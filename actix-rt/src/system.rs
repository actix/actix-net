use std::{
    cell::RefCell,
    collections::HashMap,
    future::Future,
    io,
    pin::Pin,
    sync::atomic::{AtomicUsize, Ordering},
    task::{Context, Poll},
};

use futures_core::ready;
use tokio::sync::{mpsc, oneshot};

use crate::{worker::WorkerHandle, Runtime, Worker};

static SYSTEM_COUNT: AtomicUsize = AtomicUsize::new(0);

thread_local!(
    static CURRENT: RefCell<Option<System>> = RefCell::new(None);
);

/// A manager for a per-thread distributed async runtime.
#[derive(Clone, Debug)]
pub struct System {
    id: usize,
    sys_tx: mpsc::UnboundedSender<SystemCommand>,

    /// First worker that is created as part of the System.
    worker_handle: WorkerHandle,
}


impl System {
    /// Create a new system.
    ///
    /// # Panics
    /// Panics if underlying Tokio runtime can not be created.
    #[allow(clippy::new_ret_no_self)]
    pub fn new() -> SystemRunner {
        Self::create_runtime(async {})
    }

    /// Create a new system with given initialization future.
    ///
    /// The initialization future be run to completion (blocking current thread) before the system
    /// runner is returned.
    ///
    /// # Panics
    /// Panics if underlying Tokio runtime can not be created.
    pub fn with_init(init_fut: impl Future) -> SystemRunner {
        Self::create_runtime(init_fut)
    }

    /// Constructs new system and registers it on the current thread.
    pub(crate) fn construct(
        sys_tx: mpsc::UnboundedSender<SystemCommand>,
        worker: WorkerHandle,
    ) -> Self {
        let sys = System {
            sys_tx,
            worker_handle: worker,
            id: SYSTEM_COUNT.fetch_add(1, Ordering::SeqCst),
        };

        System::set_current(sys.clone());

        sys
    }

    fn create_runtime(init_fut: impl Future) -> SystemRunner {
        let (stop_tx, stop_rx) = oneshot::channel();
        let (sys_tx, sys_rx) = mpsc::unbounded_channel();

        let rt = Runtime::new().expect("Actix (Tokio) runtime could not be created.");
        let system = System::construct(sys_tx, Worker::new_current_thread(rt.local_set()));

        // init background system worker
        let sys_worker = SystemController::new(sys_rx, stop_tx);
        rt.spawn(sys_worker);

        // run system init future
        rt.block_on(init_fut);

        SystemRunner {
            rt,
            stop_rx,
            system,
        }
    }

    /// Get current running system.
    ///
    /// # Panics
    /// Panics if no system is registered on the current thread.
    pub fn current() -> System {
        CURRENT.with(|cell| match *cell.borrow() {
            Some(ref sys) => sys.clone(),
            None => panic!("System is not running"),
        })
    }

    /// Get handle to a the System's initial [Worker].
    pub fn worker(&self) -> &WorkerHandle {
        &self.worker_handle
    }

    /// Check if there is a System registered on the current thread.
    pub fn is_registered() -> bool {
        CURRENT.with(|sys| sys.borrow().is_some())
    }

    /// Register given system on current thread.
    #[doc(hidden)]
    pub fn set_current(sys: System) {
        CURRENT.with(|cell| {
            *cell.borrow_mut() = Some(sys);
        })
    }

    /// Numeric system identifier.
    ///
    /// Useful when using multiple Systems.
    pub fn id(&self) -> usize {
        self.id
    }

    /// Stop the system (with code 0).
    pub fn stop(&self) {
        self.stop_with_code(0)
    }

    /// Stop the system with a given exit code.
    pub fn stop_with_code(&self, code: i32) {
        let _ = self.sys_tx.send(SystemCommand::Exit(code));
    }

    pub(crate) fn tx(&self) -> &mpsc::UnboundedSender<SystemCommand> {
        &self.sys_tx
    }
}

/// Runner that keeps a [System]'s event loop alive until stop message is received.
#[must_use = "A SystemRunner does nothing unless `run` is called."]
#[derive(Debug)]
pub struct SystemRunner {
    rt: Runtime,
    stop_rx: oneshot::Receiver<i32>,
    system: System,
}

impl SystemRunner {
    /// Starts event loop and will return once [System] is [stopped](System::stop).
    pub fn run(self) -> io::Result<()> {

        let SystemRunner { rt, stop_rx, .. } = self;

        // run loop
        match rt.block_on(stop_rx) {
            Ok(code) => {
                if code != 0 {
                    Err(io::Error::new(
                        io::ErrorKind::Other,
                        format!("Non-zero exit code: {}", code),
                    ))
                } else {
                    Ok(())
                }
            }

            Err(e) => Err(io::Error::new(io::ErrorKind::Other, e)),
        }
    }

    /// Runs the provided future, blocking the current thread until the future completes.
    #[inline]
    pub fn block_on<F: Future>(&self, fut: F) -> F::Output {
        self.rt.block_on(fut)
    }
}

#[derive(Debug)]
pub(crate) enum SystemCommand {
    Exit(i32),
    RegisterWorker(usize, WorkerHandle),
    DeregisterWorker(usize),
}

/// There is one `SystemController` per [System]. It runs in the background, keeping track of
/// [Worker]s and is able to distribute a system-wide stop command.
#[derive(Debug)]
pub(crate) struct SystemController {
    stop_tx: Option<oneshot::Sender<i32>>,
    cmd_rx: mpsc::UnboundedReceiver<SystemCommand>,
    workers: HashMap<usize, WorkerHandle>,
}

impl SystemController {
    pub(crate) fn new(
        cmd_rx: mpsc::UnboundedReceiver<SystemCommand>,
        stop_tx: oneshot::Sender<i32>,
    ) -> Self {
        SystemController {
            cmd_rx,
            stop_tx: Some(stop_tx),
            workers: HashMap::with_capacity(4),
        }
    }
}

impl Future for SystemController {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // process all items currently buffered in channel
        loop {
            match ready!(Pin::new(&mut self.cmd_rx).poll_recv(cx)) {
                // channel closed; no more messages can be received
                None => return Poll::Ready(()),

                // process system command
                Some(cmd) => match cmd {
                    SystemCommand::Exit(code) => {
                        // stop workers
                        for wkr in self.workers.values() {
                            wkr.stop();
                        }

                        // stop event loop
                        // will only fire once
                        if let Some(stop_tx) = self.stop_tx.take() {
                            let _ = stop_tx.send(code);
                        }
                    }

                    SystemCommand::RegisterWorker(name, hnd) => {
                        self.workers.insert(name, hnd);
                    }

                    SystemCommand::DeregisterWorker(name) => {
                        self.workers.remove(&name);
                    }
                },
            }
        }
    }
}
