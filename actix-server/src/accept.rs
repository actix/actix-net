use std::time::Duration;
use std::{io, thread};

use actix_rt::{
    time::{sleep, Instant, Sleep},
    System,
};
use log::error;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::server::Server;
use crate::socket::Listener;
use crate::worker::{Conn, WorkerHandleAccept};

struct ServerSocketInfo {
    token: usize,
    lst: Listener,
}

/// poll instance of the server.
pub(crate) struct Accept {
    handles: Vec<WorkerHandleAccept>,
    sockets: Box<[ServerSocketInfo]>,
    rx: UnboundedReceiver<Interest>,
    srv: Server,
    next: usize,
    avail: Availability,
    paused: bool,
    timeout: Pin<Box<Sleep>>,
}

pub(crate) enum Interest {
    Pause,
    Resume,
    Stop,
    WorkerIndex(usize),
    Worker(WorkerHandleAccept),
}

impl Accept {
    pub(crate) fn start(
        socks: Vec<(usize, Listener)>,
        rx: UnboundedReceiver<Interest>,
        srv: Server,
        handles: Vec<WorkerHandleAccept>,
    ) {
        // Accept runs in its own thread and would want to spawn additional futures to current
        // actix system.
        let sys = System::current();
        thread::Builder::new()
            .name("actix-server accept loop".to_owned())
            .spawn(move || {
                System::set_current(sys);

                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(async {
                        let accept = Self::new(socks, rx, srv, handles);
                        accept.await
                    });
            })
            .unwrap();
    }

    fn new(
        socks: Vec<(usize, Listener)>,
        rx: UnboundedReceiver<Interest>,
        srv: Server,
        handles: Vec<WorkerHandleAccept>,
    ) -> Self {
        let sockets = socks
            .into_iter()
            .map(|(token, lst)| ServerSocketInfo { token, lst })
            .collect();

        let mut avail = Availability::default();

        // Assume all handles are avail at construct time.
        avail.set_available_all(&handles);

        Accept {
            handles,
            sockets,
            rx,
            srv,
            next: 0,
            avail,
            paused: false,
            timeout: Box::pin(sleep(Duration::from_millis(500))),
        }
    }

    // Send connection to worker and handle error.
    fn send_connection(&mut self, conn: Conn) -> Result<(), Conn> {
        let next = self.next();
        match next.send(conn) {
            Ok(_) => {
                // Increment counter of WorkerHandle.
                // Set worker to unavailable with it hit max (Return false).
                if !next.incr_counter() {
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

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

impl Future for Accept {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        while let Poll::Ready(Some(interest)) = this.rx.poll_recv(cx) {
            match interest {
                Interest::WorkerIndex(idx) => {
                    this.avail.set_available(idx, true);
                }
                Interest::Worker(handle) => {
                    this.avail.set_available(handle.idx(), true);
                    this.handles.push(handle);
                }
                Interest::Pause => {
                    this.paused = true;
                    break;
                }
                Interest::Resume => this.paused = false,
                Interest::Stop => return Poll::Ready(()),
            }
        }

        if this.paused {
            return Poll::Pending;
        }

        let len = this.sockets.len();
        let mut idx = 0;
        while idx < len {
            'socket: loop {
                if !this.avail.available() {
                    return Poll::Pending;
                }

                let socket = &mut this.sockets[idx];

                match socket.lst.poll_accept(cx) {
                    Poll::Ready(Ok(io)) => {
                        let conn = Conn {
                            io,
                            token: socket.token,
                        };
                        this.accept_one(conn);
                    }
                    Poll::Ready(Err(ref e)) if connection_error(e) => continue 'socket,
                    Poll::Ready(Err(ref e)) => {
                        error!("Error accepting connection: {}", e);

                        let deadline = Instant::now() + Duration::from_millis(500);
                        this.timeout.as_mut().reset(deadline);
                        let _ = this.timeout.as_mut().poll(cx);

                        break 'socket;
                    }
                    Poll::Pending => break 'socket,
                };
            }
            idx += 1;
        }

        Poll::Pending
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
