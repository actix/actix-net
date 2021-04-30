use std::time::{Duration, Instant};
use std::{io, thread};

use log::{error, info};
use mio::{Interest, Poll, Token as MioToken};

use crate::builder::ServerBuilder;
use crate::server::ServerHandle;
use crate::socket::MioListener;
use crate::waker_queue::{WakerInterest, WakerQueue, WAKER_TOKEN};
use crate::worker::{Conn, ServerWorker, WorkerHandleAccept, WorkerHandleServer};

const DUR_ON_ERR: Duration = Duration::from_millis(500);

struct ServerSocketInfo {
    token: usize,

    lst: MioListener,

    /// Timeout is used to mark the deadline when this socket's listener should be registered again
    /// after an error.
    timeout: Option<Instant>,
}

/// poll instance of the server.
pub(crate) struct Accept {
    poll: Poll,
    waker_queue: WakerQueue,
    handles: Vec<WorkerHandleAccept>,
    srv: ServerHandle,
    next: usize,
    avail: Availability,
    // use the smallest duration from sockets timeout.
    timeout: Option<Duration>,
    paused: bool,
}

/// Array of u128 with every bit as marker for a worker handle's availability.
struct Availability([u128; 4]);

impl Default for Availability {
    fn default() -> Self {
        Self([0; 4])
    }
}

impl Availability {
    /// Check if any worker handle is available
    #[inline(always)]
    fn available(&self) -> bool {
        self.0.iter().any(|a| *a != 0)
    }

    /// Check if worker handle is available by index
    #[inline(always)]
    fn get_available(&self, idx: usize) -> bool {
        let (offset, idx) = Self::offset(idx);

        self.0[offset] & (1 << idx as u128) != 0
    }

    /// Set worker handle available state by index.
    fn set_available(&mut self, idx: usize, avail: bool) {
        let (offset, idx) = Self::offset(idx);

        let off = 1 << idx as u128;
        if avail {
            self.0[offset] |= off;
        } else {
            self.0[offset] &= !off
        }
    }

    /// Set all worker handle to available state.
    /// This would result in a re-check on all workers' availability.
    fn set_available_all(&mut self, handles: &[WorkerHandleAccept]) {
        handles.iter().for_each(|handle| {
            self.set_available(handle.idx(), true);
        })
    }

    /// Get offset and adjusted index of given worker handle index.
    fn offset(idx: usize) -> (usize, usize) {
        if idx < 128 {
            (0, idx)
        } else if idx < 128 * 2 {
            (1, idx - 128)
        } else if idx < 128 * 3 {
            (2, idx - 128 * 2)
        } else if idx < 128 * 4 {
            (3, idx - 128 * 3)
        } else {
            panic!("Max WorkerHandle count is 512")
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

impl Accept {
    pub(crate) fn start(
        sockets: Vec<(usize, MioListener)>,
        builder: &ServerBuilder,
    ) -> io::Result<(WakerQueue, Vec<WorkerHandleServer>)> {
        let server_handle = ServerHandle::new(builder.cmd_tx.clone());

        // construct poll instance and it's waker
        let poll = Poll::new()?;
        let waker_queue = WakerQueue::new(poll.registry())?;

        // construct workers and collect handles.
        let (handles_accept, handles_server) = (0..builder.threads)
            .map(|idx| {
                // start workers
                let factories = builder.services.iter().map(|v| v.clone_factory()).collect();

                ServerWorker::start(idx, factories, waker_queue.clone(), builder.worker_config)
            })
            .collect::<Result<Vec<_>, io::Error>>()?
            .into_iter()
            .unzip();

        let (mut accept, mut sockets) = Accept::new_with_sockets(
            poll,
            waker_queue.clone(),
            sockets,
            handles_accept,
            server_handle,
        )?;

        // Accept runs in its own thread.
        thread::Builder::new()
            .name("actix-server acceptor".to_owned())
            .spawn(move || accept.poll_with(&mut sockets))
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        // return waker and worker handle clones to server builder.
        Ok((waker_queue, handles_server))
    }

    fn new_with_sockets(
        poll: Poll,
        waker_queue: WakerQueue,
        socks: Vec<(usize, MioListener)>,
        handles: Vec<WorkerHandleAccept>,
        srv: ServerHandle,
    ) -> io::Result<(Accept, Box<[ServerSocketInfo]>)> {
        let sockets = socks
            .into_iter()
            .map(|(token, mut lst)| {
                // Start listening for incoming connections
                poll.registry()
                    .register(&mut lst, MioToken(token), Interest::READABLE)?;

                Ok(ServerSocketInfo {
                    token,
                    lst,
                    timeout: None,
                })
            })
            .collect::<Result<_, io::Error>>()?;

        let mut avail = Availability::default();

        // Assume all handles are avail at construct time.
        avail.set_available_all(&handles);

        let accept = Accept {
            poll,
            waker_queue,
            handles,
            srv,
            next: 0,
            avail,
            timeout: None,
            paused: false,
        };

        Ok((accept, sockets))
    }

    fn poll_with(&mut self, sockets: &mut [ServerSocketInfo]) {
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
                        let exit = self.handle_waker(sockets);
                        if exit {
                            info!("Accept is stopped.");
                            return;
                        }
                    }
                    _ => {
                        let token = usize::from(token);
                        self.accept(sockets, token);
                    }
                }
            }

            // check for timeout and re-register sockets.
            self.process_timeout(sockets);
        }
    }

    fn handle_waker(&mut self, sockets: &mut [ServerSocketInfo]) -> bool {
        // This is a loop because interests for command from previous version was
        // a loop that would try to drain the command channel. It's yet unknown
        // if it's necessary/good practice to actively drain the waker queue.
        loop {
            // take guard with every iteration so no new interest can be added
            // until the current task is done.
            let mut guard = self.waker_queue.guard();
            match guard.pop_front() {
                // worker notify it becomes available. we may want to recover
                // from backpressure.
                Some(WakerInterest::WorkerAvailable(idx)) => {
                    drop(guard);

                    self.avail.set_available(idx, true);

                    if !self.paused {
                        self.accept_all(sockets);
                    }
                }
                // a new worker thread is made and it's handle would be added to Accept
                Some(WakerInterest::Worker(handle)) => {
                    drop(guard);

                    self.avail.set_available(handle.idx(), true);
                    self.handles.push(handle);

                    if !self.paused {
                        self.accept_all(sockets);
                    }
                }
                Some(WakerInterest::Pause) => {
                    drop(guard);

                    if !self.paused {
                        self.paused = true;

                        self.deregister_all(sockets);
                    }
                }
                Some(WakerInterest::Resume) => {
                    drop(guard);

                    if self.paused {
                        self.paused = false;

                        sockets.iter_mut().for_each(|info| {
                            self.register_logged(info);
                        });

                        self.accept_all(sockets);
                    }
                }
                Some(WakerInterest::Stop) => {
                    if !self.paused {
                        self.deregister_all(sockets);
                    }

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

    fn process_timeout(&mut self, sockets: &mut [ServerSocketInfo]) {
        // Take old timeout as it's no use after each iteration.
        if self.timeout.take().is_some() {
            let now = Instant::now();
            sockets
                .iter_mut()
                // Only sockets that had an associated timeout were deregistered.
                .filter(|info| info.timeout.is_some())
                .for_each(|info| {
                    let inst = info.timeout.take().unwrap();

                    if now < inst {
                        // still timed out. try set new timeout.
                        info.timeout = Some(inst);
                        self.set_timeout(inst - now);
                    } else if !self.paused {
                        // timeout expired register socket again.
                        self.register_logged(info);
                    }

                    // Drop the timeout if server is paused and socket timeout is expired.
                    // When server recovers from pause it will register all sockets without
                    // a timeout value so this socket register will be delayed till then.
                });
        }
    }

    /// Update Accept timeout duration. would keep the smallest duration.
    fn set_timeout(&mut self, dur: Duration) {
        match self.timeout {
            Some(ref mut timeout) => {
                if *timeout > dur {
                    *timeout = dur;
                }
            }
            None => self.timeout = Some(dur),
        }
    }

    #[cfg(not(target_os = "windows"))]
    fn register(&self, info: &mut ServerSocketInfo) -> io::Result<()> {
        let token = MioToken(info.token);
        self.poll
            .registry()
            .register(&mut info.lst, token, Interest::READABLE)
    }

    #[cfg(target_os = "windows")]
    fn register(&self, info: &mut ServerSocketInfo) -> io::Result<()> {
        // On windows, calling register without deregister cause an error.
        // See https://github.com/actix/actix-web/issues/905
        // Calling reregister seems to fix the issue.
        let token = MioToken(info.token);
        self.poll
            .registry()
            .register(&mut info.lst, token, Interest::READABLE)
            .or_else(|_| {
                self.poll
                    .registry()
                    .reregister(&mut info.lst, token, Interest::READABLE)
            })
    }

    fn register_logged(&self, info: &mut ServerSocketInfo) {
        match self.register(info) {
            Ok(_) => info!("Resume accepting connections on {}", info.lst.local_addr()),
            Err(e) => error!("Can not register server socket {}", e),
        }
    }

    fn deregister_logged(&self, info: &mut ServerSocketInfo) {
        match self.poll.registry().deregister(&mut info.lst) {
            Ok(_) => info!("Paused accepting connections on {}", info.lst.local_addr()),
            Err(e) => {
                error!("Can not deregister server socket {}", e)
            }
        }
    }

    fn deregister_all(&self, sockets: &mut [ServerSocketInfo]) {
        // This is a best effort implementation with following limitation:
        //
        // Every ServerSocketInfo with associate timeout will be skipped and it's timeout
        // is removed in the process.
        //
        // Therefore WakerInterest::Pause followed by WakerInterest::Resume in a very short
        // gap (less than 500ms) would cause all timing out ServerSocketInfos be reregistered
        // before expected timing.
        sockets
            .iter_mut()
            // Take all timeout.
            // This is to prevent Accept::process_timer method re-register a socket afterwards.
            .map(|info| (info.timeout.take(), info))
            // Socket info with a timeout is already deregistered so skip them.
            .filter(|(timeout, _)| timeout.is_none())
            .for_each(|(_, info)| self.deregister_logged(info));
    }

    // Send connection to worker and handle error.
    fn send_connection(&mut self, conn: Conn) -> Result<(), Conn> {
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

    fn accept_one(&mut self, mut conn: Conn) {
        loop {
            let next = self.next();
            let idx = next.idx();

            if self.avail.get_available(idx) {
                match self.send_connection(conn) {
                    Ok(_) => return,
                    Err(c) => conn = c,
                }
            } else {
                self.avail.set_available(idx, false);
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

    fn accept(&mut self, sockets: &mut [ServerSocketInfo], token: usize) {
        while self.avail.available() {
            let info = &mut sockets[token];

            match info.lst.accept() {
                Ok(io) => {
                    let conn = Conn { io, token };
                    self.accept_one(conn);
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => return,
                Err(ref e) if connection_error(e) => continue,
                Err(e) => {
                    error!("Error accepting connection: {}", e);

                    // deregister listener temporary
                    self.deregister_logged(info);

                    // sleep after error. write the timeout deadline to socket info
                    // as later the poll would need it mark which socket and when
                    // it's listener should be registered again.
                    info.timeout = Some(Instant::now() + DUR_ON_ERR);
                    self.set_timeout(DUR_ON_ERR);

                    return;
                }
            };
        }
    }

    fn accept_all(&mut self, sockets: &mut [ServerSocketInfo]) {
        sockets
            .iter_mut()
            .map(|info| info.token)
            .collect::<Vec<_>>()
            .into_iter()
            .for_each(|idx| self.accept(sockets, idx))
    }

    #[inline(always)]
    fn next(&self) -> &WorkerHandleAccept {
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

#[cfg(test)]
mod test {
    use super::Availability;

    fn single(aval: &mut Availability, idx: usize) {
        aval.set_available(idx, true);
        assert!(aval.available());

        aval.set_available(idx, true);

        aval.set_available(idx, false);
        assert!(!aval.available());

        aval.set_available(idx, false);
        assert!(!aval.available());
    }

    fn multi(aval: &mut Availability, mut idx: Vec<usize>) {
        idx.iter().for_each(|idx| aval.set_available(*idx, true));

        assert!(aval.available());

        while let Some(idx) = idx.pop() {
            assert!(aval.available());
            aval.set_available(idx, false);
        }

        assert!(!aval.available());
    }

    #[test]
    fn availability() {
        let mut aval = Availability::default();

        single(&mut aval, 1);
        single(&mut aval, 128);
        single(&mut aval, 256);
        single(&mut aval, 511);

        let idx = (0..511).filter(|i| i % 3 == 0 && i % 5 == 0).collect();

        multi(&mut aval, idx);

        multi(&mut aval, (0..511).collect())
    }

    #[test]
    #[should_panic]
    fn overflow() {
        let mut aval = Availability::default();
        single(&mut aval, 512);
    }

    #[test]
    fn pin_point() {
        let mut aval = Availability::default();

        aval.set_available(438, true);

        aval.set_available(479, true);

        assert_eq!(aval.0[3], 1 << (438 - 384) | 1 << (479 - 384));
    }
}
