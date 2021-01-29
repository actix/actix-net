use std::{
    cell::RefCell,
    io,
    sync::atomic::{AtomicUsize, Ordering},
};

use tokio::sync::mpsc::UnboundedSender;

use crate::{
    arbiter::{Arbiter, SystemCommand},
    builder::{Builder, SystemRunner},
};

static SYSTEM_COUNT: AtomicUsize = AtomicUsize::new(0);

/// System is a runtime manager.
#[derive(Clone, Debug)]
pub struct System {
    id: usize,
    tx: UnboundedSender<SystemCommand>,
    arbiter: Arbiter,
    stop_on_panic: bool,
}

thread_local!(
    static CURRENT: RefCell<Option<System>> = RefCell::new(None);
);

impl System {
    /// Constructs new system and sets it as current
    pub(crate) fn construct(
        sys: UnboundedSender<SystemCommand>,
        arbiter: Arbiter,
        stop_on_panic: bool,
    ) -> Self {
        let sys = System {
            tx: sys,
            arbiter,
            stop_on_panic,
            id: SYSTEM_COUNT.fetch_add(1, Ordering::SeqCst),
        };
        System::set_current(sys.clone());
        sys
    }

    /// Build a new system with a customized tokio runtime.
    ///
    /// This allows to customize the runtime. See [`Builder`] for more information.
    pub fn builder() -> Builder {
        Builder::new()
    }

    /// Create new system.
    ///
    /// This method panics if it can not create tokio runtime
    #[allow(clippy::new_ret_no_self)]
    pub fn new(name: impl Into<String>) -> SystemRunner {
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

    pub(crate) fn tx(&self) -> &UnboundedSender<SystemCommand> {
        &self.tx
    }

    /// Return status of 'stop_on_panic' option which controls whether the System is stopped when an
    /// uncaught panic is thrown from a worker thread.
    pub(crate) fn stop_on_panic(&self) -> bool {
        self.stop_on_panic
    }

    /// Get shared reference to system arbiter.
    pub fn arbiter(&self) -> &Arbiter {
        &self.arbiter
    }

    /// This function will start tokio runtime and will finish once the `System::stop()` message
    /// is called. Function `f` is called within tokio runtime context.
    pub fn run<F>(f: F) -> io::Result<()>
    where
        F: FnOnce(),
    {
        Self::builder().run(f)
    }
}
