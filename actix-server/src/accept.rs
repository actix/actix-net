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
use crate::worker::{Conn, WorkerClient};
use crate::Token;

struct ServerSocketInfo {
    addr: SocketAddr,
    token: Token,
    sock: MioSocketListener,
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
        // Create a poll instance.
        let poll = Poll::new().unwrap_or_else(|e| panic!("Can not create mio::Poll: {}", e));

        // construct a waker queue which would wake up poll with associate extra interest types.
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

    pub fn wake_accept(&self, i: WakerInterest) {
        self.waker.wake(i);
    }

    pub(crate) fn start_accept(
        &mut self,
        socks: Vec<(Token, StdListener)>,
        workers: Vec<WorkerClient>,
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
    workers: Vec<WorkerClient>,
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
        workers: Vec<WorkerClient>,
    ) {
        let sys = System::current();

        // start accept thread
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
        workers: Vec<WorkerClient>,
        srv: Server,
        // Accept and sockets info are separated so that we can borrow mut on both at the same time
    ) -> (Accept, Slab<ServerSocketInfo>) {
        // Start accept
        let mut sockets = Slab::new();
        for (hnd_token, lst) in socks.into_iter() {
            let addr = lst.local_addr();

            let mut server = lst
                .into_mio_listener()
                .unwrap_or_else(|e| panic!("Can not set non_block on listener: {}", e));
            let entry = sockets.vacant_entry();
            let token = entry.key();

            // Start listening for incoming connections
            poll.registry()
                .register(&mut server, MioToken(token + DELTA), Interest::READABLE)
                .unwrap_or_else(|e| panic!("Can not register io: {}", e));

            entry.insert(ServerSocketInfo {
                addr,
                token: hnd_token,
                sock: server,
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
        // Create storage for events
        let mut events = mio::Events::with_capacity(128);

        loop {
            self.poll
                .poll(&mut events, None)
                .unwrap_or_else(|e| panic!("Poll error: {}", e));

            for event in events.iter() {
                let token = event.token();
                match token {
                    // This is a loop because interests for command were a loop that would try to
                    // drain the command channel. We break at first iter with other kind interests.
                    WAKER_TOKEN => 'waker: loop {
                        match self.waker.pop() {
                            Ok(i) => {
                                match i {
                                    WakerInterest::Pause => {
                                        for (_, info) in sockets.iter_mut() {
                                            if let Err(err) =
                                                self.poll.registry().deregister(&mut info.sock)
                                            {
                                                error!(
                                                    "Can not deregister server socket {}",
                                                    err
                                                );
                                            } else {
                                                info!(
                                                    "Paused accepting connections on {}",
                                                    info.addr
                                                );
                                            }
                                        }
                                    }
                                    WakerInterest::Resume => {
                                        for (token, info) in sockets.iter_mut() {
                                            if let Err(err) = self.register(token, info) {
                                                error!(
                                                    "Can not resume socket accept process: {}",
                                                    err
                                                );
                                            } else {
                                                info!(
                                                    "Accepting connections on {} has been resumed",
                                                    info.addr
                                                );
                                            }
                                        }
                                    }
                                    WakerInterest::Stop => {
                                        for (_, info) in sockets.iter_mut() {
                                            let _ =
                                                self.poll.registry().deregister(&mut info.sock);
                                        }
                                        return;
                                    }
                                    WakerInterest::Worker(worker) => {
                                        self.backpressure(&mut sockets, false);
                                        self.workers.push(worker);
                                    }
                                    // timer and notify interests need to break the loop at first iter.
                                    WakerInterest::Timer => {
                                        self.process_timer(&mut sockets);
                                        break 'waker;
                                    }
                                    WakerInterest::Notify => {
                                        self.backpressure(&mut sockets, false);
                                        break 'waker;
                                    }
                                }
                            }
                            Err(err) => match err {
                                // the waker queue is empty so we break the loop
                                WakerQueueError::Empty => break 'waker,
                                // the waker queue is closed so we return
                                WakerQueueError::Closed => {
                                    for (_, info) in sockets.iter_mut() {
                                        let _ = self.poll.registry().deregister(&mut info.sock);
                                    }
                                    return;
                                }
                            },
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

    fn process_timer(&mut self, sockets: &mut Slab<ServerSocketInfo>) {
        let now = Instant::now();
        for (token, info) in sockets.iter_mut() {
            if let Some(inst) = info.timeout.take() {
                if now > inst {
                    if let Err(err) = self.poll.registry().register(
                        &mut info.sock,
                        MioToken(token + DELTA),
                        Interest::READABLE,
                    ) {
                        error!("Can not register server socket {}", err);
                    } else {
                        info!("Resume accepting connections on {}", info.addr);
                    }
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

    fn backpressure(&mut self, sockets: &mut Slab<ServerSocketInfo>, on: bool) {
        if self.backpressure {
            if !on {
                self.backpressure = false;
                for (token, info) in sockets.iter_mut() {
                    if let Err(err) = self.register(token, info) {
                        error!("Can not resume socket accept process: {}", err);
                    } else {
                        info!("Accepting connections on {} has been resumed", info.addr);
                    }
                }
            }
        } else if on {
            self.backpressure = true;
            for (_, info) in sockets.iter_mut() {
                let _ = self.poll.registry().deregister(&mut info.sock);
            }
        }
    }

    fn accept_one(&mut self, sockets: &mut Slab<ServerSocketInfo>, mut msg: Conn) {
        if self.backpressure {
            while !self.workers.is_empty() {
                match self.workers[self.next].send(msg) {
                    Ok(_) => (),
                    Err(tmp) => {
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
                self.next = (self.next + 1) % self.workers.len();
                break;
            }
        } else {
            let mut idx = 0;
            while idx < self.workers.len() {
                idx += 1;
                if self.workers[self.next].available() {
                    match self.workers[self.next].send(msg) {
                        Ok(_) => {
                            self.next = (self.next + 1) % self.workers.len();
                            return;
                        }
                        Err(tmp) => {
                            self.srv.worker_faulted(self.workers[self.next].idx);
                            msg = tmp;
                            self.workers.swap_remove(self.next);
                            if self.workers.is_empty() {
                                error!("No workers");
                                self.backpressure(sockets, true);
                                return;
                            } else if self.workers.len() <= self.next {
                                self.next = 0;
                            }
                            continue;
                        }
                    }
                }
                self.next = (self.next + 1) % self.workers.len();
            }
            // enable backpressure
            self.backpressure(sockets, true);
            self.accept_one(sockets, msg);
        }
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
                        error!("Error accepting connection: {}", e);
                        if let Err(err) = self.poll.registry().deregister(&mut info.sock) {
                            error!("Can not deregister server socket {}", err);
                        }

                        // sleep after error
                        info.timeout = Some(Instant::now() + Duration::from_millis(500));

                        let w = self.waker.clone();
                        System::current().arbiter().send(Box::pin(async move {
                            sleep_until(Instant::now() + Duration::from_millis(510)).await;
                            w.wake(WakerInterest::Timer);
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
