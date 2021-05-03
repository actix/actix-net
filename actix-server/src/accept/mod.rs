#[cfg(not(_loom))]
use std::thread;

#[cfg(_loom)]
use loom::thread;

mod acceptable;
mod availability;

pub use acceptable::Acceptable;

use std::{io, time::Duration};

use actix_rt::{
    time::{sleep, Instant},
    System,
};
use log::{error, info};
use mio::{Poll, Registry, Token};
use tokio::sync::oneshot;

use availability::Availability;

use crate::server::Server;
use crate::waker_queue::{WakerInterest, WakerQueue, WAKER_TOKEN};
use crate::worker::{Conn, WorkerHandleAccept};

/// Accept loop would live with `ServerBuilder`.
///
/// It's tasked with construct `Poll` instance and `WakerQueue` which would be distributed to
/// `Accept` and `Worker`.
///
/// It would also listen to `ServerCommand` and push interests to `WakerQueue`.
pub(crate) struct AcceptLoop<A: Acceptable> {
    srv: Option<Server>,
    poll: Option<Poll>,
    waker: WakerQueue<A::Connection>,
}

impl<A> AcceptLoop<A>
where
    A: Acceptable + Send + 'static,
{
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

    pub(crate) fn waker_owned(&self) -> WakerQueue<A::Connection> {
        self.waker.clone()
    }

    pub fn wake(&self, i: WakerInterest<A::Connection>) {
        self.waker.wake(i);
    }

    pub(crate) fn start(
        &mut self,
        socks: Vec<(usize, A)>,
        handles: Vec<WorkerHandleAccept<A::Connection>>,
    ) {
        let srv = self.srv.take().expect("Can not re-use AcceptInfo");
        let poll = self.poll.take().unwrap();
        let waker = self.waker.clone();

        Accept::start(poll, waker, socks, srv, handles);
    }
}

/// poll instance of the server.
struct Accept<A: Acceptable> {
    poll: Poll,
    source: Box<[Source<A>]>,
    waker: WakerQueue<A::Connection>,
    handles: Vec<WorkerHandleAccept<A::Connection>>,
    srv: Server,
    next: usize,
    avail: Availability,
    paused: bool,
}

struct Source<A: Acceptable> {
    token: usize,

    acceptable: A,

    /// Timeout is used to mark the deadline when this socket's listener should be registered again
    /// after an error.
    timeout: Option<Instant>,
}

impl<A: Acceptable> Source<A> {
    #[inline(always)]
    fn register(&mut self, registry: &Registry) {
        let token = Token(self.token);
        match self.acceptable.register(registry, token) {
            Ok(_) => info!("Start accepting connections on {:?}", &self.acceptable),
            Err(e) => error!("Can not register {}", e),
        }
    }

    #[inline(always)]
    fn deregister(&mut self, registry: &Registry) {
        match self.acceptable.deregister(registry) {
            Ok(_) => info!("Paused accepting connections on {:?}", &self.acceptable),
            Err(e) => error!("Can not deregister {}", e),
        }
    }
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

impl<A> Accept<A>
where
    A: Acceptable + Send + 'static,
{
    pub(crate) fn start(
        poll: Poll,
        waker: WakerQueue<A::Connection>,
        source: Vec<(usize, A)>,
        srv: Server,
        handles: Vec<WorkerHandleAccept<A::Connection>>,
    ) {
        // Accept runs in its own thread and would want to spawn additional futures to current
        // actix system.
        let sys = System::current();
        thread::Builder::new()
            .name("actix-server acceptor".to_owned())
            .spawn(move || {
                System::set_current(sys);
                let source = source
                    .into_iter()
                    .map(|(token, mut lst)| {
                        // Start listening for incoming connections
                        lst.register(poll.registry(), Token(token))
                            .unwrap_or_else(|e| panic!("Can not register io: {}", e));

                        Source {
                            token,
                            acceptable: lst,
                            timeout: None,
                        }
                    })
                    .collect();

                let mut avail = Availability::default();

                // Assume all handles are avail at construct time.
                handles.iter().for_each(|handle| {
                    avail.set_available(handle.idx(), true);
                });

                let accept = Accept {
                    poll,
                    source,
                    waker,
                    handles,
                    srv,
                    next: 0,
                    avail,
                    paused: false,
                };

                accept.poll();
            })
            .unwrap();
    }

    fn poll(mut self) {
        let mut events = mio::Events::with_capacity(128);

        loop {
            if let Err(e) = self.poll.poll(&mut events, None) {
                match e.kind() {
                    io::ErrorKind::Interrupted => {}
                    _ => panic!("Poll error: {}", e),
                }
            }

            for event in events.iter() {
                let token = event.token();
                match token {
                    WAKER_TOKEN => {
                        let exit = self.handle_waker();
                        if exit {
                            info!("Accept is stopped.");
                            return;
                        }
                    }
                    _ => {
                        let token = usize::from(token);
                        self.accept(token);
                    }
                }
            }
        }
    }

    fn handle_waker(&mut self) -> bool {
        // This is a loop because interests for command from previous version was
        // a loop that would try to drain the command channel. It's yet unknown
        // if it's necessary/good practice to actively drain the waker queue.
        loop {
            // take guard with every iteration so no new interest can be added
            // until the current task is done.
            let mut guard = self.waker.guard();
            match guard.pop_front() {
                // worker notify it becomes available.
                Some(WakerInterest::WorkerAvailable(idx)) => {
                    drop(guard);

                    self.avail.set_available(idx, true);

                    if !self.paused {
                        self.accept_all();
                    }
                }
                // a new worker thread is made and it's handle would be added to Accept
                Some(WakerInterest::Worker(handle)) => {
                    drop(guard);

                    self.avail.set_available(handle.idx(), true);
                    self.handles.push(handle);

                    if !self.paused {
                        self.accept_all();
                    }
                }
                // got timer interest and it's time to try register socket(s) again
                Some(WakerInterest::Timer) => {
                    drop(guard);

                    self.process_timer()
                }
                Some(WakerInterest::Pause) => {
                    drop(guard);

                    if !self.paused {
                        self.paused = true;

                        self.deregister_all();
                    }
                }
                Some(WakerInterest::Resume) => {
                    drop(guard);

                    if self.paused {
                        self.paused = false;

                        self.register_all();

                        self.accept_all();
                    }
                }
                Some(WakerInterest::Stop(AcceptorStop { graceful, tx })) => {
                    drop(guard);

                    if !self.paused {
                        self.deregister_all();
                    }

                    // Collect oneshot receiver if WorkerHandle::stop returns it.
                    let res = self
                        .handles
                        .iter()
                        .filter_map(|handle| handle.stop(graceful))
                        .collect::<Vec<_>>();

                    let _ = tx.send(res);

                    // TODO: Should try to drain backlog?

                    return true;
                }
                // waker queue is drained
                None => {
                    // Reset the WakerQueue before break so it does not grow infinitely
                    WakerQueue::reset(&mut guard);

                    return false;
                }
            }
        }
    }

    // Send connection to worker and handle error.
    fn send_connection(
        &mut self,
        conn: Conn<A::Connection>,
    ) -> Result<(), Conn<A::Connection>> {
        let next = self.next();
        match next.send(conn) {
            Ok(_) => {
                // Increment counter of WorkerHandle.
                // Set worker to unavailable with it hit max (Return false).
                if !next.inc_counter() {
                    let idx = next.idx();
                    self.avail.set_available(idx, false);
                }
                self.set_next();
                Ok(())
            }
            Err(conn) => {
                // Worker thread is error and could be gone.
                // Remove worker handle and notify `ServerBuilder`.
                self.remove_next();

                if self.handles.is_empty() {
                    error!("No workers");
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

    fn accept_one(&mut self, mut conn: Conn<A::Connection>) {
        loop {
            let next = self.next();
            let idx = next.idx();

            if self.avail.get_available(idx) {
                match self.send_connection(conn) {
                    Ok(_) => return,
                    Err(c) => conn = c,
                }
            } else {
                self.set_next();

                if !self.avail.available() {
                    while let Err(c) = self.send_connection(conn) {
                        conn = c;
                    }
                    return;
                }
            }
        }
    }

    fn accept(&mut self, token: usize) {
        while self.avail.available() {
            let source = &mut self.source[token];

            match source.acceptable.accept() {
                Ok(Some(io)) => {
                    let conn = Conn { token, io };
                    self.accept_one(conn);
                }
                Ok(None) => continue,
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => return,
                Err(ref e) if connection_error(e) => continue,
                Err(e) => {
                    error!("Error accepting connection: {}", e);

                    // deregister listener temporary
                    source.deregister(self.poll.registry());

                    // sleep after error. write the timeout to socket info as later
                    // the poll would need it mark which socket and when it's
                    // listener should be registered
                    source.timeout = Some(Instant::now() + Duration::from_millis(500));

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

    fn accept_all(&mut self) {
        self.source
            .iter_mut()
            .map(|info| info.token)
            .collect::<Vec<_>>()
            .into_iter()
            .for_each(|idx| self.accept(idx))
    }

    fn process_timer(&mut self) {
        let now = Instant::now();

        let registry = self.poll.registry();
        let paused = self.paused;

        self.source
            .iter_mut()
            // Only sockets that had an associated timeout were deregistered.
            .filter(|info| info.timeout.is_some())
            .for_each(|info| {
                let inst = info.timeout.take().unwrap();

                if now < inst {
                    info.timeout = Some(inst);
                } else if !paused {
                    info.register(registry);
                }

                // Drop the timeout if server is paused and socket timeout is expired.
                // When server recovers from pause it will register all sockets without
                // a timeout value so this socket register will be delayed till then.
            });
    }

    fn register_all(&mut self) {
        let registry = self.poll.registry();
        self.source.iter_mut().for_each(|info| {
            info.register(registry);
        });
    }

    fn deregister_all(&mut self) {
        let registry = self.poll.registry();
        // This is a best effort implementation with following limitation:
        //
        // Every ServerSocketInfo with associate timeout will be skipped and it's timeout
        // is removed in the process.
        //
        // Therefore WakerInterest::Pause followed by WakerInterest::Resume in a very short
        // gap (less than 500ms) would cause all timing out ServerSocketInfos be reregistered
        // before expected timing.
        self.source
            .iter_mut()
            // Take all timeout.
            // This is to prevent Accept::process_timer method re-register a socket afterwards.
            .map(|info| (info.timeout.take(), info))
            // Socket info with a timeout is already deregistered so skip them.
            .filter(|(timeout, _)| timeout.is_none())
            .for_each(|(_, info)| info.deregister(registry));
    }

    #[inline(always)]
    fn next(&self) -> &WorkerHandleAccept<A::Connection> {
        &self.handles[self.next]
    }

    /// Set next worker handle that would accept connection.
    #[inline(always)]
    fn set_next(&mut self) {
        self.next = (self.next + 1) % self.handles.len();
    }

    /// Remove next worker handle that fail to accept connection.
    fn remove_next(&mut self) {
        let handle = self.handles.swap_remove(self.next);
        let idx = handle.idx();
        // A message is sent to `ServerBuilder` future to notify it a new worker
        // should be made.
        self.srv.worker_faulted(idx);
        self.avail.set_available(idx, false);
    }
}

pub(crate) struct AcceptorStop {
    graceful: bool,
    tx: oneshot::Sender<Vec<oneshot::Receiver<bool>>>,
}

impl AcceptorStop {
    pub(crate) fn new(
        graceful: bool,
    ) -> (Self, oneshot::Receiver<Vec<oneshot::Receiver<bool>>>) {
        let (tx, rx) = oneshot::channel();

        let this = Self { graceful, tx };

        (this, rx)
    }
}
