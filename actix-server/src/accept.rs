use std::time::Duration;
use std::{io, thread};

use actix_rt::time::{sleep_until, Instant};
use actix_rt::System;
use log::{error, info};
use mio::{Interest, Poll, Token as MioToken};
use slab::Slab;

use crate::server::Server;
use crate::socket::{MioSocketListener, SocketAddr, StdListener};
use crate::waker_queue::{WakerInterest, WakerQueue, WakerQueueError, WAKER_TOKEN};
use crate::worker::{Conn, WorkerHandle};
use crate::Token;

struct ServerSocketInfo {
    addr: SocketAddr,
    token: Token,
    sock: MioSocketListener,
    // timeout is used to mark the time this socket should be reregistered after an error.
    timeout: Option<Instant>,
}

/// Accept loop would live with `ServerBuilder`.
///
/// It's tasked with construct `Poll` instance and `WakerQueue` which would be distributed to
/// `Accept` and `WorkerClient` accordingly.
///
/// It would also listen to `ServerCommand` and push interests to `WakerQueue`.
pub(crate) struct AcceptLoop {
    srv: Option<Server>,
    poll: Option<Poll>,
    waker: WakerQueue,
}

impl AcceptLoop {
    pub fn new(srv: Server) -> Self {
        let poll = Poll::new().unwrap_or_else(|e| panic!("Can not create mio::Poll: {}", e));
        let waker = WakerQueue::with_capacity(poll.registry(), 128).unwrap();

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
        socks: Vec<(Token, StdListener)>,
        workers: Vec<WorkerHandle>,
    ) {
        let srv = self.srv.take().expect("Can not re-use AcceptInfo");
        let poll = self.poll.take().unwrap();
        let waker = self.waker.clone();

        Accept::start(poll, waker, socks, srv, workers);
    }
}

/// poll instance of the server.
struct Accept {
    poll: Poll,
    waker: WakerQueue,
    workers: Vec<WorkerHandle>,
    srv: Server,
    next: usize,
    backpressure: bool,
}

const DELTA: usize = 100;

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
        socks: Vec<(Token, StdListener)>,
        srv: Server,
        workers: Vec<WorkerHandle>,
    ) {
        // Accept runs in its own thread and would want to spawn additional futures to current
        // actix system.
        let sys = System::current();
        thread::Builder::new()
            .name("actix-server accept loop".to_owned())
            .spawn(move || {
                System::set_current(sys);
                let (mut accept, sockets) =
                    Accept::new_with_sockets(poll, waker, socks, workers, srv);
                accept.poll_with(sockets);
            })
            .unwrap();
    }

    fn new_with_sockets(
        poll: Poll,
        waker: WakerQueue,
        socks: Vec<(Token, StdListener)>,
        workers: Vec<WorkerHandle>,
        srv: Server,
    ) -> (Accept, Slab<ServerSocketInfo>) {
        let mut sockets = Slab::new();
        for (hnd_token, lst) in socks.into_iter() {
            let addr = lst.local_addr();

            let mut sock = lst
                .into_mio_listener()
                .unwrap_or_else(|e| panic!("Can not set non_block on listener: {}", e));
            let entry = sockets.vacant_entry();
            let token = entry.key();

            // Start listening for incoming connections
            poll.registry()
                .register(&mut sock, MioToken(token + DELTA), Interest::READABLE)
                .unwrap_or_else(|e| panic!("Can not register io: {}", e));

            entry.insert(ServerSocketInfo {
                addr,
                token: hnd_token,
                sock,
                timeout: None,
            });
        }

        let accept = Accept {
            poll,
            waker,
            workers,
            srv,
            next: 0,
            backpressure: false,
        };

        (accept, sockets)
    }

    fn poll_with(&mut self, mut sockets: Slab<ServerSocketInfo>) {
        let mut events = mio::Events::with_capacity(128);

        loop {
            self.poll
                .poll(&mut events, None)
                .unwrap_or_else(|e| panic!("Poll error: {}", e));

            for event in events.iter() {
                let token = event.token();
                match token {
                    // This is a loop because interests for command from previous version was a
                    // loop that would try to drain the command channel. It's yet unknown if it's
                    // necessary/good practice to actively drain the waker queue.
                    WAKER_TOKEN => 'waker: loop {
                        match self.waker.pop() {
                            // worker notify it's availability has change. we maybe want to enter
                            // backpressure or recover from one.
                            Ok(WakerInterest::Notify) => {
                                self.maybe_backpressure(&mut sockets, false);
                            }
                            Ok(WakerInterest::Pause) => {
                                sockets.iter_mut().for_each(|(_, info)| {
                                    if let Err(err) = self.deregister(info) {
                                        error!("Can not deregister server socket {}", err);
                                    } else {
                                        info!("Paused accepting connections on {}", info.addr);
                                    }
                                });
                            }
                            Ok(WakerInterest::Resume) => {
                                sockets.iter_mut().for_each(|(token, info)| {
                                    self.register_logged(token, info);
                                });
                            }
                            Ok(WakerInterest::Stop) => {
                                return self.deregister_all(&mut sockets);
                            }
                            // a new worker thread is made and it's client would be added to Accept
                            Ok(WakerInterest::Worker(worker)) => {
                                // maybe we want to recover from a backpressure.
                                self.maybe_backpressure(&mut sockets, false);
                                self.workers.push(worker);
                            }
                            // got timer interest and it's time to try register socket(s) again.
                            Ok(WakerInterest::Timer) => self.process_timer(&mut sockets),
                            Err(WakerQueueError::Empty) => break 'waker,
                            Err(WakerQueueError::Closed) => {
                                return self.deregister_all(&mut sockets);
                            }
                        }
                    },
                    _ => {
                        let token = usize::from(token);
                        if token < DELTA {
                            continue;
                        }
                        self.accept(&mut sockets, token - DELTA);
                    }
                }
            }
        }
    }

    fn process_timer(&self, sockets: &mut Slab<ServerSocketInfo>) {
        let now = Instant::now();
        for (token, info) in sockets.iter_mut() {
            // only the sockets have an associate timeout value was de registered.
            if let Some(inst) = info.timeout.take() {
                if now > inst {
                    self.register_logged(token, info);
                } else {
                    info.timeout = Some(inst);
                }
            }
        }
    }

    #[cfg(not(target_os = "windows"))]
    fn register(&self, token: usize, info: &mut ServerSocketInfo) -> io::Result<()> {
        self.poll.registry().register(
            &mut info.sock,
            MioToken(token + DELTA),
            Interest::READABLE,
        )
    }

    #[cfg(target_os = "windows")]
    fn register(&self, token: usize, info: &mut ServerSocketInfo) -> io::Result<()> {
        // On windows, calling register without deregister cause an error.
        // See https://github.com/actix/actix-web/issues/905
        // Calling reregister seems to fix the issue.
        self.poll
            .registry()
            .register(
                &mut info.sock,
                mio::Token(token + DELTA),
                Interest::READABLE,
            )
            .or_else(|_| {
                self.poll.registry().reregister(
                    &mut info.sock,
                    mio::Token(token + DELTA),
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
        self.poll.registry().deregister(&mut info.sock)
    }

    fn deregister_all(&self, sockets: &mut Slab<ServerSocketInfo>) {
        sockets.iter_mut().for_each(|(_, info)| {
            let _ = self.deregister(info);
        });
    }

    fn maybe_backpressure(&mut self, sockets: &mut Slab<ServerSocketInfo>, on: bool) {
        if self.backpressure {
            if !on {
                self.backpressure = false;
                for (token, info) in sockets.iter_mut() {
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
            while !self.workers.is_empty() {
                match self.workers[self.next].send(msg) {
                    Ok(_) => {
                        self.set_next();
                        break;
                    }
                    Err(tmp) => {
                        // worker lost contact and could be gone. a message is sent to
                        // `ServerBuilder` future to notify it a new worker should be made.
                        // after that remove the fault worker and enter backpressure if necessary.
                        self.srv.worker_faulted(self.workers[self.next].idx);
                        msg = tmp;
                        self.workers.swap_remove(self.next);
                        if self.workers.is_empty() {
                            error!("No workers");
                            return;
                        } else if self.workers.len() <= self.next {
                            self.next = 0;
                        }
                        continue;
                    }
                }
            }
        } else {
            let mut idx = 0;
            while idx < self.workers.len() {
                idx += 1;
                if self.workers[self.next].available() {
                    match self.workers[self.next].send(msg) {
                        Ok(_) => {
                            self.set_next();
                            return;
                        }
                        // worker lost contact and could be gone. a message is sent to
                        // `ServerBuilder` future to notify it a new worker should be made.
                        // after that remove the fault worker and enter backpressure if necessary.
                        Err(tmp) => {
                            self.srv.worker_faulted(self.workers[self.next].idx);
                            msg = tmp;
                            self.workers.swap_remove(self.next);
                            if self.workers.is_empty() {
                                error!("No workers");
                                self.maybe_backpressure(sockets, true);
                                return;
                            } else if self.workers.len() <= self.next {
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

    // set next worker that would accept work.
    fn set_next(&mut self) {
        self.next = (self.next + 1) % self.workers.len();
    }

    fn accept(&mut self, sockets: &mut Slab<ServerSocketInfo>, token: usize) {
        loop {
            let msg = if let Some(info) = sockets.get_mut(token) {
                match info.sock.accept() {
                    Ok(Some((io, addr))) => Conn {
                        io,
                        token: info.token,
                        peer: Some(addr),
                    },
                    Ok(None) => return,
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => return,
                    Err(ref e) if connection_error(e) => continue,
                    Err(e) => {
                        // deregister socket temporary
                        error!("Error accepting connection: {}", e);
                        if let Err(err) = self.poll.registry().deregister(&mut info.sock) {
                            error!("Can not deregister server socket {}", err);
                        }

                        // sleep after error. write the timeout to socket info as later the poll
                        // would need it mark which socket and when it should be registered.
                        info.timeout = Some(Instant::now() + Duration::from_millis(500));

                        // after the sleep a Timer interest is sent to Accept Poll
                        let waker = self.waker.clone();
                        System::current().arbiter().send(Box::pin(async move {
                            sleep_until(Instant::now() + Duration::from_millis(510)).await;
                            waker.wake(WakerInterest::Timer);
                        }));
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
