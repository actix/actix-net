use std::time::{Duration, Instant};
use std::{io, thread};

use log::{error, info};
use mio::{Interest, Poll, Token as MioToken};
use slab::Slab;

use crate::server_handle::ServerHandle;
use crate::socket::{MioListener, SocketAddr};
use crate::waker_queue::{WakerInterest, WakerQueue, WAKER_TOKEN};
use crate::worker::{Conn, WorkerHandle};
use crate::Token;

const DUR_ON_ERR: Duration = Duration::from_millis(500);

struct ServerSocketInfo {
    // addr for socket. mainly used for logging.
    addr: SocketAddr,
    // be ware this is the crate token for identify socket and should not be confused with
    // mio::Token
    token: Token,
    lst: MioListener,
    // mark the deadline when this socket's listener should be registered again
    timeout_deadline: Option<Instant>,
}

/// poll instance of the server.
pub(crate) struct Accept {
    poll: Poll,
    waker_queue: WakerQueue,
    handles: Vec<WorkerHandle>,
    srv: ServerHandle,
    next: usize,
    backpressure: bool,
    // poll time duration.
    // use the smallest duration from sockets timeout_deadline.
    timeout: Option<Duration>,
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
    pub(crate) fn start<F>(
        sockets: Vec<(Token, MioListener)>,
        server_handle: ServerHandle,
        worker_factory: F,
    ) -> WakerQueue
    where
        F: FnOnce(&WakerQueue) -> Vec<WorkerHandle>,
    {
        // construct poll instance and it's waker
        let poll = Poll::new().unwrap_or_else(|e| panic!("Can not create `mio::Poll`: {}", e));
        let waker_queue = WakerQueue::new(poll.registry())
            .unwrap_or_else(|e| panic!("Can not create `mio::Waker`: {}", e));
        let waker_clone = waker_queue.clone();

        // construct workers and collect handles.
        let handles = worker_factory(&waker_queue);

        // Accept runs in its own thread.
        thread::Builder::new()
            .name("actix-server acceptor".to_owned())
            .spawn(move || {
                let (mut accept, sockets) = Accept::new_with_sockets(
                    poll,
                    waker_queue,
                    sockets,
                    handles,
                    server_handle,
                );
                accept.poll_with(sockets);
            })
            .unwrap();

        // return waker to server builder.
        waker_clone
    }

    fn new_with_sockets(
        poll: Poll,
        waker_queue: WakerQueue,
        socks: Vec<(Token, MioListener)>,
        handles: Vec<WorkerHandle>,
        srv: ServerHandle,
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
                timeout_deadline: None,
            });
        }

        let accept = Accept {
            poll,
            waker_queue,
            handles,
            srv,
            next: 0,
            backpressure: false,
            timeout: None,
        };

        (accept, sockets)
    }

    fn poll_with(&mut self, mut sockets: Slab<ServerSocketInfo>) {
        let mut events = mio::Events::with_capacity(128);

        loop {
            if let Err(e) = self.poll.poll(&mut events, self.timeout) {
                match e.kind() {
                    std::io::ErrorKind::Interrupted => {
                        // check for timeout and re-register sockets.
                        self.process_timeout(&mut sockets);
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
                        let mut guard = self.waker_queue.guard();
                        match guard.pop_front() {
                            // worker notify it becomes available. we may want to recover
                            // from  backpressure.
                            Some(WakerInterest::WorkerAvailable) => {
                                drop(guard);
                                self.maybe_backpressure(&mut sockets, false);
                            }
                            // a new worker thread is made and it's handle would be added
                            // to Accept
                            Some(WakerInterest::Worker(handle)) => {
                                drop(guard);
                                // maybe we want to recover from a backpressure.
                                self.maybe_backpressure(&mut sockets, false);
                                self.handles.push(handle);
                            }
                            Some(WakerInterest::Pause) => {
                                drop(guard);
                                sockets.iter_mut().for_each(|(_, info)| {
                                    match self.deregister(info) {
                                        Ok(_) => info!(
                                            "Paused accepting connections on {}",
                                            info.addr
                                        ),
                                        Err(e) => {
                                            error!("Can not deregister server socket {}", e)
                                        }
                                    }
                                });
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
                            // waker queue is drained.
                            None => {
                                // Reset the WakerQueue before break so it does not grow
                                // infinitely.
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

            // check for timeout and re-register sockets.
            self.process_timeout(&mut sockets);
        }
    }

    fn process_timeout(&mut self, sockets: &mut Slab<ServerSocketInfo>) {
        // take old timeout as it's no use after each iteration.
        if self.timeout.take().is_some() {
            let now = Instant::now();
            sockets.iter_mut().for_each(|(token, info)| {
                // only the ServerSocketInfo have an associate timeout value was de registered.
                if let Some(inst) = info.timeout_deadline {
                    // timeout expired register socket again.
                    if now >= inst {
                        info.timeout_deadline = None;
                        self.register_logged(token, info);
                    } else {
                        // still timed out. try set new timeout.
                        let dur = inst - now;
                        self.set_timeout(dur);
                    }
                }
            });
        }
    }

    // update Accept timeout duration. would keep the smallest duration.
    fn set_timeout(&mut self, dur: Duration) {
        match self.timeout {
            Some(timeout) => {
                if timeout > dur {
                    self.timeout = Some(dur);
                }
            }
            None => self.timeout = Some(dur),
        }
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

    fn deregister_all(&self, sockets: &mut Slab<ServerSocketInfo>) {
        sockets.iter_mut().for_each(|(_, info)| {
            info!("Accepting connections on {} has been paused", info.addr);
            let _ = self.deregister(info);
        });
    }

    fn maybe_backpressure(&mut self, sockets: &mut Slab<ServerSocketInfo>, on: bool) {
        if self.backpressure {
            if !on {
                self.backpressure = false;
                for (token, info) in sockets.iter_mut() {
                    if info.timeout_deadline.is_some() {
                        // socket will attempt to re-register itself when its timeout completes
                        continue;
                    }
                    self.register_logged(token, info);
                }
            }
        } else if on {
            self.backpressure = true;
            self.deregister_all(sockets);
        }
    }

    fn accept_one(&mut self, sockets: &mut Slab<ServerSocketInfo>, mut msg: Conn) {
        if self.backpressure {
            while !self.handles.is_empty() {
                match self.handles[self.next].send(msg) {
                    Ok(_) => {
                        self.set_next();
                        break;
                    }
                    Err(tmp) => {
                        // worker lost contact and could be gone. a message is sent to
                        // `ServerBuilder` future to notify it a new worker should be made.
                        // after that remove the fault worker.
                        self.srv.worker_faulted(self.handles[self.next].idx);
                        msg = tmp;
                        self.handles.swap_remove(self.next);
                        if self.handles.is_empty() {
                            error!("No workers");
                            return;
                        } else if self.handles.len() <= self.next {
                            self.next = 0;
                        }
                        continue;
                    }
                }
            }
        } else {
            let mut idx = 0;
            while idx < self.handles.len() {
                idx += 1;
                if self.handles[self.next].available() {
                    match self.handles[self.next].send(msg) {
                        Ok(_) => {
                            self.set_next();
                            return;
                        }
                        // worker lost contact and could be gone. a message is sent to
                        // `ServerBuilder` future to notify it a new worker should be made.
                        // after that remove the fault worker and enter backpressure if necessary.
                        Err(tmp) => {
                            self.srv.worker_faulted(self.handles[self.next].idx);
                            msg = tmp;
                            self.handles.swap_remove(self.next);
                            if self.handles.is_empty() {
                                error!("No workers");
                                self.maybe_backpressure(sockets, true);
                                return;
                            } else if self.handles.len() <= self.next {
                                self.next = 0;
                            }
                            continue;
                        }
                    }
                }
                self.set_next();
            }
            // enable backpressure
            self.maybe_backpressure(sockets, true);
            self.accept_one(sockets, msg);
        }
    }

    // set next worker handle that would accept work.
    fn set_next(&mut self) {
        self.next = (self.next + 1) % self.handles.len();
    }

    fn accept(&mut self, sockets: &mut Slab<ServerSocketInfo>, token: usize) {
        loop {
            let msg = if let Some(info) = sockets.get_mut(token) {
                match info.lst.accept() {
                    Ok(Some((io, addr))) => Conn {
                        io,
                        token: info.token,
                        peer: Some(addr),
                    },
                    Ok(None) => return,
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => return,
                    Err(ref e) if connection_error(e) => continue,
                    Err(e) => {
                        // deregister listener temporary
                        error!("Error accepting connection: {}", e);
                        if let Err(err) = self.deregister(info) {
                            error!("Can not deregister server socket {}", err);
                        }

                        // sleep after error. write the timeout deadline to socket info
                        // as later the poll would need it mark which socket and when
                        // it's listener should be registered again.
                        info.timeout_deadline = Some(Instant::now() + DUR_ON_ERR);
                        self.set_timeout(DUR_ON_ERR);

                        return;
                    }
                }
            } else {
                return;
            };

            self.accept_one(sockets, msg);
        }
    }
}
