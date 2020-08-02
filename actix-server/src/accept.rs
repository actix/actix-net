use std::sync::mpsc as sync_mpsc;
use std::time::Duration;
use std::{io, thread};

use actix_rt::time::{delay_until, Instant};
use actix_rt::System;
use log::{error, info};
use slab::Slab;

use crate::server::Server;
use crate::socket::{SocketAddr, SocketListener, StdListener};
use crate::worker::{Conn, WorkerClient};
use crate::Token;

pub(crate) enum Command {
    Pause,
    Resume,
    Stop,
    Worker(WorkerClient),
}

struct ServerSocketInfo {
    addr: SocketAddr,
    token: Token,
    sock: SocketListener,
    timeout: Option<Instant>,
}

#[derive(Clone)]
pub(crate) struct AcceptNotify(mio::SetReadiness);

impl AcceptNotify {
    pub(crate) fn new(ready: mio::SetReadiness) -> Self {
        AcceptNotify(ready)
    }

    pub(crate) fn notify(&self) {
        let _ = self.0.set_readiness(mio::Ready::readable());
    }
}

impl Default for AcceptNotify {
    fn default() -> Self {
        AcceptNotify::new(mio::Registration::new2().1)
    }
}

pub(crate) struct AcceptLoop {
    cmd_reg: Option<mio::Registration>,
    cmd_ready: mio::SetReadiness,
    notify_reg: Option<mio::Registration>,
    notify_ready: mio::SetReadiness,
    tx: sync_mpsc::Sender<Command>,
    rx: Option<sync_mpsc::Receiver<Command>>,
    srv: Option<Server>,
}

impl AcceptLoop {
    pub fn new(srv: Server) -> AcceptLoop {
        let (tx, rx) = sync_mpsc::channel();
        let (cmd_reg, cmd_ready) = mio::Registration::new2();
        let (notify_reg, notify_ready) = mio::Registration::new2();

        AcceptLoop {
            tx,
            cmd_ready,
            cmd_reg: Some(cmd_reg),
            notify_ready,
            notify_reg: Some(notify_reg),
            rx: Some(rx),
            srv: Some(srv),
        }
    }

    pub fn send(&self, msg: Command) {
        let _ = self.tx.send(msg);
        let _ = self.cmd_ready.set_readiness(mio::Ready::readable());
    }

    pub fn get_notify(&self) -> AcceptNotify {
        AcceptNotify::new(self.notify_ready.clone())
    }

    pub(crate) fn start(
        &mut self,
        socks: Vec<(Token, StdListener)>,
        workers: Vec<WorkerClient>,
    ) {
        let srv = self.srv.take().expect("Can not re-use AcceptInfo");

        Accept::start(
            self.rx.take().expect("Can not re-use AcceptInfo"),
            self.cmd_reg.take().expect("Can not re-use AcceptInfo"),
            self.notify_reg.take().expect("Can not re-use AcceptInfo"),
            socks,
            srv,
            workers,
        );
    }
}

struct Accept {
    poll: mio::Poll,
    rx: sync_mpsc::Receiver<Command>,
    sockets: Slab<ServerSocketInfo>,
    workers: Vec<WorkerClient>,
    srv: Server,
    timer: (mio::Registration, mio::SetReadiness),
    next_worker_ix: usize,
    backpressure: bool,
}

const DELTA: usize = 100;
const CMD: mio::Token = mio::Token(0);
const TIMER: mio::Token = mio::Token(1);
const NOTIFY: mio::Token = mio::Token(2);

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

// One-shot enum, indicates how to repond to commands
enum ShouldAbort {
    Abort,
    Continue,
}

impl Accept {
    #![allow(clippy::too_many_arguments)]
    pub(crate) fn start(
        rx: sync_mpsc::Receiver<Command>,
        cmd_reg: mio::Registration,
        notify_reg: mio::Registration,
        socks: Vec<(Token, StdListener)>,
        srv: Server,
        workers: Vec<WorkerClient>,
    ) {
        // start accepting events (within separate thread)
        let sys = System::current();

        let _ = thread::Builder::new()
            .name("actix-server accept loop".to_owned())
            .spawn(move || {
                System::set_current(sys);
                let mut accept = Accept::new(rx, socks, workers, srv);

                // Start listening for incoming commands
                if let Err(err) = accept.poll.register(
                    &cmd_reg,
                    CMD,
                    mio::Ready::readable(),
                    mio::PollOpt::edge(),
                ) {
                    panic!("Can not register Registration: {}", err);
                }

                // Start listening for notify updates
                if let Err(err) = accept.poll.register(
                    &notify_reg,
                    NOTIFY,
                    mio::Ready::readable(),
                    mio::PollOpt::edge(),
                ) {
                    panic!("Can not register Registration: {}", err);
                }

                // Start core accept loop. Blocks indefinitely except in case of error
                accept.poll();
            });
    }

    fn new(
        rx: sync_mpsc::Receiver<Command>,
        socks: Vec<(Token, StdListener)>,
        workers: Vec<WorkerClient>,
        srv: Server,
    ) -> Accept {
        // Create a poll instance
        let poll = match mio::Poll::new() {
            Ok(poll) => poll,
            Err(err) => panic!("Can not create mio::Poll: {}", err),
        };

        // Start accept
        let mut sockets = Slab::new();
        for (hnd_token, lst) in socks.into_iter() {
            let addr = lst.local_addr();

            let server = lst.into_listener();
            let entry = sockets.vacant_entry();
            let token = entry.key();

            // Start listening for incoming connections
            if let Err(err) = poll.register(
                &server,
                mio::Token(token + DELTA),
                mio::Ready::readable(),
                mio::PollOpt::edge(),
            ) {
                panic!("Can not register io: {}", err);
            }

            entry.insert(ServerSocketInfo {
                addr,
                token: hnd_token,
                sock: server,
                timeout: None,
            });
        }

        // Timer
        let (tm, tmr) = mio::Registration::new2();
        if let Err(err) =
            poll.register(&tm, TIMER, mio::Ready::readable(), mio::PollOpt::edge())
        {
            panic!("Can not register Registration: {}", err);
        }

        Accept {
            poll,
            rx,
            sockets,
            workers,
            srv,
            next_worker_ix: 0,
            timer: (tm, tmr),
            backpressure: false,
        }
    }

    // Core acceptor logic. Receive notifications from the event loop and respond.
    // In particular, receive notifications of pending connections, and accept them.
    fn poll(&mut self) {
        // Create storage for events
        let mut events = mio::Events::with_capacity(128);

        loop {
            // block here, waiting to receive events from the `tokio` event loop
            if let Err(err) = self.poll.poll(&mut events, None) {
                panic!("Poll error: {}", err);
            }

            // now process the events
            for event in events.iter() {
                let token = event.token();
                // as well as responding to socket events,
                // we also use `mio` as a messaging layer for
                // actix events
                match token {
                    CMD => {
                        // There is a pending message from the server
                        match self.process_cmd() {
                            ShouldAbort::Abort => return,
                            ShouldAbort::Continue => continue,
                        }
                    }
                    TIMER => self.process_timer(),
                    NOTIFY => {
                        // A message from a worker thread indicating that it is
                        // available again - therefore remove backpressure, if any
                        self.set_backpressure(false)
                    }
                    _ => {
                        // any other token indicates a pending connection - accept it
                        let token = usize::from(token);
                        if token < DELTA {
                            continue;
                        }
                        self.accept(token - DELTA);
                    }
                }
            }
            // all events processed - loop!
        }
    }

    fn process_timer(&mut self) {
        // This function is triggered after an IO error. During error recovery
        // the affected socket is de-registered, and after some timeout
        // we must re-register it
        let now = Instant::now();
        for (token, info) in self.sockets.iter_mut() {
            if let Some(inst) = info.timeout.take() {
                if now > inst {
                    if let Err(err) = self.poll.register(
                        &info.sock,
                        mio::Token(token + DELTA),
                        mio::Ready::readable(),
                        mio::PollOpt::edge(),
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

    /// Process messages received from server
    fn process_cmd(&mut self) -> ShouldAbort {
        loop {
            match self.rx.try_recv() {
                Ok(cmd) => match cmd {
                    Command::Pause => {
                        for (_, info) in self.sockets.iter_mut() {
                            if let Err(err) = self.poll.deregister(&info.sock) {
                                error!("Can not deregister server socket {}", err);
                            } else {
                                info!("Paused accepting connections on {}", info.addr);
                            }
                        }
                    }
                    Command::Resume => {
                        for (token, info) in self.sockets.iter() {
                            if let Err(err) = self.register(token, info) {
                                error!("Can not resume socket accept process: {}", err);
                            } else {
                                info!(
                                    "Accepting connections on {} has been resumed",
                                    info.addr
                                );
                            }
                        }
                    }
                    Command::Stop => {
                        for (_, info) in self.sockets.iter() {
                            let _ = self.poll.deregister(&info.sock);
                        }
                        return ShouldAbort::Abort;
                    }
                    Command::Worker(worker) => {
                        self.set_backpressure(false);
                        self.workers.push(worker);
                    }
                },
                Err(err) => match err {
                    sync_mpsc::TryRecvError::Empty => break,
                    sync_mpsc::TryRecvError::Disconnected => {
                        for (_, info) in self.sockets.iter() {
                            let _ = self.poll.deregister(&info.sock);
                        }
                        return ShouldAbort::Abort;
                    }
                },
            }
        }
        ShouldAbort::Continue
    }

    #[cfg(not(target_os = "windows"))]
    fn register(&self, token: usize, info: &ServerSocketInfo) -> io::Result<()> {
        self.poll.register(
            &info.sock,
            mio::Token(token + DELTA),
            mio::Ready::readable(),
            mio::PollOpt::edge(),
        )
    }

    #[cfg(target_os = "windows")]
    fn register(&self, token: usize, info: &ServerSocketInfo) -> io::Result<()> {
        // On windows, calling register without deregister cause an error.
        // See https://github.com/actix/actix-web/issues/905
        // Calling reregister seems to fix the issue.
        self.poll
            .register(
                &info.sock,
                mio::Token(token + DELTA),
                mio::Ready::readable(),
                mio::PollOpt::edge(),
            )
            .or_else(|_| {
                self.poll.reregister(
                    &info.sock,
                    mio::Token(token + DELTA),
                    mio::Ready::readable(),
                    mio::PollOpt::edge(),
                )
            })
    }

    /// While backpressure is enabled, we will not accept any
    /// new connections (but existing ones will be served)
    fn set_backpressure(&mut self, on: bool) {
        if self.backpressure == on {
            // already set -> no op
            return;
        }
        self.backpressure = on;
        if self.backpressure {
            // stop being notified of pending connections
            for (_, info) in self.sockets.iter() {
                let _ = self.poll.deregister(&info.sock);
            }
        } else {
            // resume being notified of pending connections
            for (token, info) in self.sockets.iter() {
                if let Err(err) = self.register(token, info) {
                    error!("Can not resume socket accept process: {}", err);
                } else {
                    info!("Accepting connections on {} has been resumed", info.addr);
                }
            }
        }
    }

    fn accept_one(&mut self, mut conn: Conn) {
        // we have an incomming connection, we must send it to a worker

        if self.backpressure {
            while !self.workers.is_empty() {
                match self.workers[self.next_worker_ix].send(conn) {
                    Ok(_) => (),
                    Err(tmp) => {
                        // the receiving end of the channel is closed,
                        // probably because the worker thread has crashed

                        // recover the connection and notify the server
                        conn = tmp;
                        self.srv
                            .worker_faulted(self.workers[self.next_worker_ix].idx);

                        self.workers.swap_remove(self.next_worker_ix);
                        if self.workers.is_empty() {
                            error!("No workers");
                            return;
                        } else if self.workers.len() <= self.next_worker_ix {
                            self.next_worker_ix = 0;
                        }
                        continue;
                    }
                }
                self.next_worker_ix = (self.next_worker_ix + 1) % self.workers.len();
                break;
            }
        } else {
            // We iterate through our workers, starting from
            // `self.next_worker_ix`, and try to find one that is not busy
            let mut idx = 0;
            while idx < self.workers.len() {
                idx += 1;
                if self.workers[self.next_worker_ix].is_available() {
                    // worker has indicated that it is available, so
                    // send the connection through a channel to the worker thread
                    match self.workers[self.next_worker_ix].send(conn) {
                        Ok(()) => {
                            // connection sent to worker, bump the index and we're done
                            self.next_worker_ix =
                                (self.next_worker_ix + 1) % self.workers.len();
                            return;
                        }
                        Err(tmp) => {
                            // the receiving end of the channel is closed,
                            // probably because the worker thread has crashed

                            // recover the connection and notify the server
                            conn = tmp;
                            self.srv
                                .worker_faulted(self.workers[self.next_worker_ix].idx);

                            // discard the crashed worker
                            self.workers.swap_remove(self.next_worker_ix);

                            if self.workers.is_empty() {
                                // all workers have crashed! Drop the connection,
                                // we'll try to recover next time
                                error!("No workers");
                                self.set_backpressure(true);
                                return;
                            } else if self.workers.len() <= self.next_worker_ix {
                                // self.next_worker_ix now points out-of-bounds, so reset it
                                self.next_worker_ix = 0;
                            }
                            continue;
                        }
                    }
                }
                self.next_worker_ix = (self.next_worker_ix + 1) % self.workers.len();
            }
            // No workers are available. Enable backpressure and try again
            self.set_backpressure(true);
            self.accept_one(conn);
        }
    }

    fn accept(&mut self, token: usize) {
        // This is the core 'accept' loop and is critical for overall performace.
        // The overall logic is: We have received a token the mio event loop
        // This could indicate // FIXME
        // Now we need to do something useful with it
        loop {
            let conn = if let Some(info) = self.sockets.get_mut(token) {
                // We have already registered this token, which means
                // that the event relates to a request that is has already
                // been accepted and is in the middle of being handled by actix

                match info.sock.accept() {
                    Ok(Some((io, addr))) => {
                        // connection accepted (happy path)
                        Conn {
                            io,
                            token: info.token,
                            peer: Some(addr),
                        }
                    }
                    Ok(None) => {
                        // Only reachable for unix domain sockets. No waiting connection
                        // so nothing to be done. Yield to the event loop
                        return;
                    }
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                        // Socket not ready - yield to the event loop
                        return;
                    }
                    Err(ref e) if connection_error(e) => {
                        // connection error, retry the socket
                        continue;
                    }
                    Err(e) => {
                        // some other (fatal) IO error
                        // We will attempt to recover by deregistering the socket
                        // with mio, then after a short pause sending a notification
                        // to re-register the socket
                        error!("Error accepting connection: {}", e);
                        if let Err(err) = self.poll.deregister(&info.sock) {
                            error!("Can not deregister server socket {}", err);
                        }

                        info.timeout = Some(Instant::now() + Duration::from_millis(500));

                        // create and run a future which will sleep for a short period
                        // then trigger a mio event
                        let r = self.timer.1.clone();
                        System::current().arbiter().send(Box::pin(async move {
                            delay_until(Instant::now() + Duration::from_millis(510)).await;
                            let _ = r.set_readiness(mio::Ready::readable());
                        }));
                        return;
                    }
                }
            } else {
                // no socket associated with the token, implies the token is
                // stale in some way. Nothing to do so yield to the event loop
                return;
            };

            self.accept_one(conn);
        }
    }
}
