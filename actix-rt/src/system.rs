use std::{
    borrow::Cow,
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

use crate::{
    builder::{Builder, SystemRunner},
    worker::Worker,
};

static SYSTEM_COUNT: AtomicUsize = AtomicUsize::new(0);

/// System is a runtime manager.
#[derive(Clone, Debug)]
pub struct System {
    id: usize,
    tx: mpsc::UnboundedSender<SystemCommand>,
    worker: Worker,
}

thread_local!(
    static CURRENT: RefCell<Option<System>> = RefCell::new(None);
);

impl System {
    /// Constructs new system and sets it as current
    pub(crate) fn construct(sys: mpsc::UnboundedSender<SystemCommand>, worker: Worker) -> Self {
        let sys = System {
            tx: sys,
            worker,
            id: SYSTEM_COUNT.fetch_add(1, Ordering::SeqCst),
        };
        System::set_current(sys.clone());
        sys
    }

    /// Build a new system with a customized Tokio runtime.
    ///
    /// This allows to customize the runtime. See [`Builder`] for more information.
    pub fn builder() -> Builder {
        Builder::new()
    }

    /// Create new system.
    ///
    /// # Panics
    /// Panics if underlying Tokio runtime can not be created.
    #[allow(clippy::new_ret_no_self)]
    pub fn new(name: impl Into<Cow<'static, str>>) -> SystemRunner {
        Self::builder().name(name).build()
    }

    /// Get current running system.
    pub fn current() -> System {
        CURRENT.with(|cell| match *cell.borrow() {
            Some(ref sys) => sys.clone(),
            None => panic!("System is not running"),
        })
    }

    /// Check if current system has started.
    pub fn is_set() -> bool {
        CURRENT.with(|cell| cell.borrow().is_some())
    }

    /// Set current running system.
    #[doc(hidden)]
    pub fn set_current(sys: System) {
        CURRENT.with(|s| {
            *s.borrow_mut() = Some(sys);
        })
    }

    /// Execute function with system reference.
    pub fn with_current<F, R>(f: F) -> R
    where
        F: FnOnce(&System) -> R,
    {
        CURRENT.with(|cell| match *cell.borrow() {
            Some(ref sys) => f(sys),
            None => panic!("System is not running"),
        })
    }

    /// Numeric system ID.
    pub fn id(&self) -> usize {
        self.id
    }

    /// Stop the system (with code 0).
    pub fn stop(&self) {
        self.stop_with_code(0)
    }

    /// Stop the system with a particular exit code.
    pub fn stop_with_code(&self, code: i32) {
        let _ = self.tx.send(SystemCommand::Exit(code));
    }

    pub(crate) fn tx(&self) -> &mpsc::UnboundedSender<SystemCommand> {
        &self.tx
    }

    /// Get shared reference to system arbiter.
    pub fn arbiter(&self) -> &Worker {
        &self.worker
    }

    /// This function will start Tokio runtime and will finish once the `System::stop()` message
    /// is called. Function `f` is called within Tokio runtime context.
    pub fn run<F>(f: F) -> io::Result<()>
    where
        F: FnOnce(),
    {
        Self::builder().run(f)
    }
}

#[derive(Debug)]
pub(crate) enum SystemCommand {
    Exit(i32),
    RegisterArbiter(usize, Worker),
    DeregisterArbiter(usize),
}

#[derive(Debug)]
pub(crate) struct SystemWorker {
    stop: Option<oneshot::Sender<i32>>,
    commands: mpsc::UnboundedReceiver<SystemCommand>,
    workers: HashMap<usize, Worker>,
}

impl SystemWorker {
    pub(crate) fn new(
        commands: mpsc::UnboundedReceiver<SystemCommand>,
        stop: oneshot::Sender<i32>,
    ) -> Self {
        SystemWorker {
            commands,
            stop: Some(stop),
            workers: HashMap::new(),
        }
    }
}

impl Future for SystemWorker {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // process all items currently buffered in channel
        loop {
            match ready!(Pin::new(&mut self.commands).poll_recv(cx)) {
                // channel closed; no more messages can be received
                None => return Poll::Ready(()),

                // process system command
                Some(cmd) => match cmd {
                    SystemCommand::Exit(code) => {
                        // stop arbiters
                        for arb in self.workers.values() {
                            arb.stop();
                        }
                        // stop event loop
                        if let Some(stop) = self.stop.take() {
                            let _ = stop.send(code);
                        }
                    }
                    SystemCommand::RegisterArbiter(name, hnd) => {
                        self.workers.insert(name, hnd);
                    }
                    SystemCommand::DeregisterArbiter(name) => {
                        self.workers.remove(&name);
                    }
                },
            }
        }
    }
}
