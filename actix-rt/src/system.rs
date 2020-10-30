use std::cell::RefCell;
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};

use futures_channel::mpsc::UnboundedSender;

use crate::arbiter::{Arbiter, SystemCommand};
use crate::builder::{Builder, SystemRunner};
use crate::runtime::ExecFactory;
use crate::ActixExec;

static SYSTEM_COUNT: AtomicUsize = AtomicUsize::new(0);

/// System is a runtime manager.
#[derive(Clone, Debug)]
pub struct System {
    id: usize,
    sys: UnboundedSender<SystemCommand>,
    arbiter: Arbiter,
    stop_on_panic: bool,
}

thread_local!(
    static CURRENT: RefCell<Option<System>> = RefCell::new(None);
);

impl System {
    #[allow(clippy::new_ret_no_self)]
    /// Create new system.
    ///
    /// This method panics if it can not create tokio runtime
    pub fn new<T: Into<String>>(name: T) -> SystemRunner<ActixExec> {
        Self::new_with::<ActixExec, T>(name)
    }

    /// This function will start tokio runtime and will finish once the
    /// `System::stop()` message get called.
    /// Function `f` get called within tokio runtime context.
    pub fn run<F>(f: F) -> io::Result<()>
    where
        F: FnOnce() + 'static,
    {
        Self::run_with::<ActixExec, F>(f)
    }

    /// Get current running system.
    pub fn current() -> System {
        CURRENT.with(|cell| match *cell.borrow() {
            Some(ref sys) => sys.clone(),
            None => panic!("System is not running"),
        })
    }

    /// Check if current system is set, i.e., as already been started.
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

    /// System id
    pub fn id(&self) -> usize {
        self.id
    }

    /// Stop the system
    pub fn stop(&self) {
        self.stop_with_code(0)
    }

    /// Stop the system with a particular exit code.
    pub fn stop_with_code(&self, code: i32) {
        let _ = self.sys.unbounded_send(SystemCommand::Exit(code));
    }

    pub(crate) fn sys(&self) -> &UnboundedSender<SystemCommand> {
        &self.sys
    }

    /// Return status of 'stop_on_panic' option which controls whether the System is stopped when an
    /// uncaught panic is thrown from a worker thread.
    pub fn stop_on_panic(&self) -> bool {
        self.stop_on_panic
    }

    /// System arbiter
    pub fn arbiter(&self) -> &Arbiter {
        &self.arbiter
    }

    /// Build a new system with custom executor.
    pub fn builder<E: ExecFactory>() -> Builder<E> {
        Builder::new()
    }

    #[allow(clippy::new_ret_no_self)]
    /// Create new system with custom runtime.
    ///
    /// This method panics if it can not create the runtime type.
    pub fn new_with<E: ExecFactory, T: Into<String>>(name: T) -> SystemRunner<E> {
        Self::builder().name(name).build()
    }

    pub fn run_with<E, F>(f: F) -> io::Result<()>
    where
        E: ExecFactory,
        F: FnOnce() + 'static,
    {
        Self::builder::<E>().run(f)
    }
}

impl System {
    /// Constructs new system and sets it as current
    pub(crate) fn construct(
        sys: UnboundedSender<SystemCommand>,
        arbiter: Arbiter,
        stop_on_panic: bool,
    ) -> Self {
        let sys = System {
            sys,
            arbiter,
            stop_on_panic,
            id: SYSTEM_COUNT.fetch_add(1, Ordering::SeqCst),
        };
        System::set_current(sys.clone());
        sys
    }
}
