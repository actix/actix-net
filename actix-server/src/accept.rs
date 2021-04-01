use std::time::Duration;
use std::{io, thread};

use actix_rt::{
    time::{sleep, Instant},
    System,
};
use log::{error, info};
use mio::{Interest, Poll, Token as MioToken};
use slab::Slab;

use crate::server::Server;
use crate::socket::{MioListener, SocketAddr};
use crate::waker_queue::{WakerInterest, WakerQueue, WAKER_TOKEN};
use crate::worker::{Conn, WorkerHandle};
use crate::Token;

struct ServerSocketInfo {
    /// Address of socket. Mainly used for logging.
    addr: SocketAddr,

    /// Beware this is the crate token for identify socket and should not be confused
    /// with `mio::Token`.
    token: Token,

    lst: MioListener,

    /// Timeout is used to mark the deadline when this socket's listener should be registered again
    /// after an error.
    timeout: Option<Instant>,
}

/// Accept loop would live with `ServerBuilder`.
///
/// It's tasked with construct `Poll` instance and `WakerQueue` which would be distributed to
/// `Accept` and `Worker`.
///
/// It would also listen to `ServerCommand` and push interests to `WakerQueue`.
pub(crate) struct AcceptLoop {
    srv: Option<Server>,
    poll: Option<Poll>,
    waker: WakerQueue,
}

impl AcceptLoop {
    pub fn new(srv: Server) -> Self {
        let poll = Poll::new().unwrap_or_else(|e| panic!("Can not create `mio::Poll`: {}", e));
        let waker = WakerQueue::new(poll.registry())
            .unwrap_or_else(|e| panic!("Can not create `mio::Waker`: {}", e));

        Self {
            srv: Some(srv),
            poll: Some(poll),
            waker,
        }
    }

    pub(crate) fn waker_owned(&self) -> WakerQueue {
        self.waker.clone()
    }

    pub fn wake(&self, i: WakerInterest) {
        self.waker.wake(i);
    }

    pub(crate) fn start(
        &mut self,
        socks: Vec<(Token, MioListener)>,
        handles: Vec<WorkerHandle>,
    ) {
        let srv = self.srv.take().expect("Can not re-use AcceptInfo");
        let poll = self.poll.take().unwrap();
        let waker = self.waker.clone();

        Accept::start(poll, waker, socks, srv, handles);
    }
}

/// poll instance of the server.
struct Accept {
    poll: Poll,
    waker: WakerQueue,
    handles: Vec<WorkerHandle>,
    srv: Server,
    next: usize,
    backpressure: bool,
}

/// This function defines errors that are per-connection. Which basically
/// means that if we get this error from `accept()` system call it means
/// next connection might be ready to be accepted.
///
/// All other errors will incur a timeout before next `accept()` is performed.
/// The timeout is useful to handle resource exhaustion errors like ENFILE
/// and EMFILE. Otherwise, could enter into tight loop.
fn connection_error(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::ConnectionRefused
        || e.kind() == io::ErrorKind::ConnectionAborted
        || e.kind() == io::ErrorKind::ConnectionReset
}

impl Accept {
    pub(crate) fn start(
        poll: Poll,
        waker: WakerQueue,
        socks: Vec<(Token, MioListener)>,
        srv: Server,
        handles: Vec<WorkerHandle>,
    ) {
        // Accept runs in its own thread and would want to spawn additional futures to current
        // actix system.
        let sys = System::current();
        thread::Builder::new()
            .name("actix-server accept loop".to_owned())
            .spawn(move || {
                System::set_current(sys);
                let (mut accept, sockets) =
                    Accept::new_with_sockets(poll, waker, socks, handles, srv);
                accept.poll_with(sockets);
            })
            .unwrap();
    }

    fn new_with_sockets(
        poll: Poll,
        waker: WakerQueue,
        socks: Vec<(Token, MioListener)>,
        handles: Vec<WorkerHandle>,
        srv: Server,
    ) -> (Accept, Slab<ServerSocketInfo>) {
        let mut sockets = Slab::new();
        for (hnd_token, mut lst) in socks.into_iter() {
            let addr = lst.local_addr();

            let entry = sockets.vacant_entry();
            let token = entry.key();

            // Start listening for incoming connections
            poll.registry()
                .register(&mut lst, MioToken(token), Interest::READABLE)
                .unwrap_or_else(|e| panic!("Can not register io: {}", e));

            entry.insert(ServerSocketInfo {
                addr,
                token: hnd_token,
                lst,
                timeout: None,
            });
        }

        let accept = Accept {
            poll,
            waker,
            handles,
            srv,
            next: 0,
            backpressure: false,
        };

        (accept, sockets)
    }

    fn poll_with(&mut self, mut sockets: Slab<ServerSocketInfo>) {
        let mut events = mio::Events::with_capacity(128);

        loop {
            if let Err(e) = self.poll.poll(&mut events, None) {
                match e.kind() {
                    std::io::ErrorKind::Interrupted => {
                        continue;
                    }
                    _ => {
                        panic!("Poll error: {}", e);
                    }
                }
            }

            for event in events.iter() {
                let token = event.token();
                match token {
                    // This is a loop because interests for command from previous version was
                    // a loop that would try to drain the command channel. It's yet unknown
                    // if it's necessary/good practice to actively drain the waker queue.
                    WAKER_TOKEN => 'waker: loop {
                        // take guard with every iteration so no new interest can be added
                        // until the current task is done.
                        let mut guard = self.waker.guard();
                        match guard.pop_front() {
                            // worker notify it becomes available. we may want to recover
                            // from backpressure.
                            Some(WakerInterest::WorkerAvailable) => {
                                drop(guard);
                                self.maybe_backpressure(&mut sockets, false);
                            }
                            // a new worker thread is made and it's handle would be added to Accept
                            Some(WakerInterest::Worker(handle)) => {
                                drop(guard);
                                // maybe we want to recover from a backpressure.
                                self.maybe_backpressure(&mut sockets, false);
                                self.handles.push(handle);
                            }
                            // got timer interest and it's time to try register socket(s) again
                            Some(WakerInterest::Timer) => {
                                drop(guard);
                                self.process_timer(&mut sockets)
                            }
                            Some(WakerInterest::Pause) => {
                                drop(guard);
                                self.deregister_all(&mut sockets);
                            }
                            Some(WakerInterest::Resume) => {
                                drop(guard);
                                sockets.iter_mut().for_each(|(token, info)| {
                                    self.register_logged(token, info);
                                });
                            }
                            Some(WakerInterest::Stop) => {
                                return self.deregister_all(&mut sockets);
                            }
                            // waker queue is drained
                            None => {
                                // Reset the WakerQueue before break so it does not grow infinitely
                                WakerQueue::reset(&mut guard);
                                break 'waker;
                            }
                        }
                    },
                    _ => {
                        let token = usize::from(token);
                        self.accept(&mut sockets, token);
                    }
                }
            }
        }
    }

    fn process_timer(&self, sockets: &mut Slab<ServerSocketInfo>) {
        let now = Instant::now();
        sockets
            .iter_mut()
            // Only sockets that had an associated timeout were deregistered.
            .filter(|(_, info)| info.timeout.is_some())
            .for_each(|(token, info)| {
                let inst = info.timeout.take().unwrap();

                if now < inst {
                    info.timeout = Some(inst);
                } else if !self.backpressure {
                    self.register_logged(token, info);
                }

                // Drop the timeout if server is in backpressure and socket timeout is expired.
                // When server recovers from backpressure it will register all sockets without
                // a timeout value so this socket register will be delayed till then.
            });
    }

    #[cfg(not(target_os = "windows"))]
    fn register(&self, token: usize, info: &mut ServerSocketInfo) -> io::Result<()> {
        self.poll
            .registry()
            .register(&mut info.lst, MioToken(token), Interest::READABLE)
    }

    #[cfg(target_os = "windows")]
    fn register(&self, token: usize, info: &mut ServerSocketInfo) -> io::Result<()> {
        // On windows, calling register without deregister cause an error.
        // See https://github.com/actix/actix-web/issues/905
        // Calling reregister seems to fix the issue.
        self.poll
            .registry()
            .register(&mut info.lst, mio::Token(token), Interest::READABLE)
            .or_else(|_| {
                self.poll.registry().reregister(
                    &mut info.lst,
                    mio::Token(token),
                    Interest::READABLE,
                )
            })
    }

    fn register_logged(&self, token: usize, info: &mut ServerSocketInfo) {
        match self.register(token, info) {
            Ok(_) => info!("Resume accepting connections on {}", info.addr),
            Err(e) => error!("Can not register server socket {}", e),
        }
    }

    fn deregister(&self, info: &mut ServerSocketInfo) -> io::Result<()> {
        self.poll.registry().deregister(&mut info.lst)
    }

    fn deregister_logged(&self, info: &mut ServerSocketInfo) {
        match self.deregister(info) {
            Ok(_) => info!("Paused accepting connections on {}", info.addr),
            Err(e) => {
                error!("Can not deregister server socket {}", e)
            }
        }
    }

    fn deregister_all(&self, sockets: &mut Slab<ServerSocketInfo>) {
        sockets.iter_mut().for_each(|(_, info)| {
            self.deregister_logged(info);
        });
    }

    fn maybe_backpressure(&mut self, sockets: &mut Slab<ServerSocketInfo>, on: bool) {
        // Only operate when server is in a different backpressure than the given flag.
        if self.backpressure != on {
            if on {
                self.backpressure = true;
                // TODO: figure out if timing out sockets can be safely de-registered twice.
                self.deregister_all(sockets);
            } else {
                self.backpressure = false;
                sockets
                    .iter_mut()
                    // Only operate on sockets without associated timeout.
                    // Sockets with it will attempt to re-register when their timeout expires.
                    .filter(|(_, info)| info.timeout.is_none())
                    .for_each(|(token, info)| self.register_logged(token, info));
            }
        }
    }

    fn accept_one(&mut self, sockets: &mut Slab<ServerSocketInfo>, mut conn: Conn) {
        if self.backpressure {
            // send_connection would remove fault worker from handles.
            // worst case here is conn get dropped after all handles are gone.
            while !self.handles.is_empty() {
                match self.send_connection(sockets, conn) {
                    Ok(_) => return,
                    Err(c) => conn = c,
                }
            }
        } else {
            // Do one round and try to send conn to all workers until it succeed.
            // Start from self.next.
            let mut idx = 0;
            while idx < self.handles.len() {
                idx += 1;
                if self.handles[self.next].available() {
                    match self.send_connection(sockets, conn) {
                        Ok(_) => return,
                        Err(c) => conn = c,
                    }
                } else {
                    self.set_next();
                }
            }
            // Sending Conn failed due to either all workers are in error or not available.
            // Enter backpressure state and try again.
            self.maybe_backpressure(sockets, true);
            self.accept_one(sockets, conn);
        }
    }

    // Set next worker handle that would accept work.
    fn set_next(&mut self) {
        self.next = (self.next + 1) % self.handles.len();
    }

    // Send connection to worker and handle error.
    fn send_connection(
        &mut self,
        sockets: &mut Slab<ServerSocketInfo>,
        conn: Conn,
    ) -> Result<(), Conn> {
        match self.handles[self.next].send(conn) {
            Ok(_) => {
                self.set_next();
                Ok(())
            }
            Err(conn) => {
                // worker lost contact and could be gone. a message is sent to
                // `ServerBuilder` future to notify it a new worker should be made.
                // after that remove the fault worker and enter backpressure if necessary.
                self.srv.worker_faulted(self.handles[self.next].idx);
                self.handles.swap_remove(self.next);
                if self.handles.is_empty() {
                    error!("No workers");
                    self.maybe_backpressure(sockets, true);
                    // All workers are gone and Conn is nowhere to be sent.
                    // Treat this situation as Ok and drop Conn.
                    return Ok(());
                } else if self.handles.len() <= self.next {
                    self.next = 0;
                }
                Err(conn)
            }
        }
    }

    fn accept(&mut self, sockets: &mut Slab<ServerSocketInfo>, token: usize) {
        loop {
            let info = sockets
                .get_mut(token)
                .expect("ServerSocketInfo is removed from Slab");

            match info.lst.accept() {
                Ok(io) => {
                    let msg = Conn {
                        io,
                        token: info.token,
                    };
                    self.accept_one(sockets, msg);
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => return,
                Err(ref e) if connection_error(e) => continue,
                Err(e) => {
                    error!("Error accepting connection: {}", e);

                    // deregister listener temporary
                    self.deregister_logged(info);

                    // sleep after error. write the timeout to socket info as later
                    // the poll would need it mark which socket and when it's
                    // listener should be registered
                    info.timeout = Some(Instant::now() + Duration::from_millis(500));

                    // after the sleep a Timer interest is sent to Accept Poll
                    let waker = self.waker.clone();
                    System::current().arbiter().spawn(async move {
                        sleep(Duration::from_millis(510)).await;
                        waker.wake(WakerInterest::Timer);
                    });

                    return;
                }
            };
        }
    }
}
