use std::cell::RefCell;
use std::future::Future;
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};

use futures_channel::mpsc::UnboundedSender;
use tokio::task::LocalSet;

use crate::arbiter::{Arbiter, SystemCommand};
use crate::builder::{Builder, SystemRunner};

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

    /// Build a new system with a customized tokio runtime.
    ///
    /// This allows to customize the runtime. See struct level docs on
    /// `Builder` for more information.
    pub fn builder() -> Builder {
        Builder::new()
    }

    #[allow(clippy::new_ret_no_self)]
    /// Create new system.
    ///
    /// This method panics if it can not create tokio runtime
    pub fn new<T: Into<String>>(name: T) -> SystemRunner {
        Self::builder().name(name).build()
    }

    /// Create new system using provided tokio `LocalSet`.
    ///
    /// This method panics if it can not spawn system arbiter
    ///
    /// Note: This method uses provided `LocalSet` to create a `System` future only.
    /// All the [`Arbiter`]s will be started in separate threads using their own tokio `Runtime`s.
    /// It means that using this method currently it is impossible to make `actix-rt` work in the
    /// alternative `tokio` `Runtime`s (e.g. provided by [`tokio_compat`]).
    ///
    /// [`tokio_compat`]: https://crates.io/crates/tokio-compat
    ///
    /// # Examples
    ///
    /// ```
    /// use tokio::{runtime::Runtime, task::LocalSet};
    /// use actix_rt::System;
    /// use futures_util::future::try_join_all;
    ///
    /// async fn run_application() {
    ///     let first_task = tokio::spawn(async {
    ///         // ...
    /// #        println!("One task");
    /// #        Ok::<(),()>(())
    ///     });
    ///
    ///     let second_task = tokio::spawn(async {
    ///         // ...
    /// #       println!("Another task");
    /// #       Ok::<(),()>(())
    ///     });
    ///
    ///     try_join_all(vec![first_task, second_task])
    ///         .await
    ///         .expect("Some of the futures finished unexpectedly");
    /// }
    ///
    ///
    /// let mut runtime = tokio::runtime::Builder::new()
    ///     .core_threads(2)
    ///     .enable_all()
    ///     .threaded_scheduler()
    ///     .build()
    ///     .unwrap();
    ///
    ///
    /// let actix_system_task = LocalSet::new();
    /// let sys = System::run_in_tokio("actix-main-system", &actix_system_task);
    /// actix_system_task.spawn_local(sys);
    ///
    /// let rest_operations = run_application();
    /// runtime.block_on(actix_system_task.run_until(rest_operations));
    /// ```
    pub fn run_in_tokio<T: Into<String>>(
        name: T,
        local: &LocalSet,
    ) -> impl Future<Output = io::Result<()>> {
        Self::builder()
            .name(name)
            .build_async(local)
            .run_nonblocking()
    }

    /// Consume the provided tokio Runtime and start the `System` in it.
    /// This method will create a `LocalSet` object and occupy the current thread
    /// for the created `System` exclusively. All the other asynchronous tasks that
    /// should be executed as well must be aggregated into one future, provided as the last
    /// argument to this method.
    ///
    /// Note: This method uses provided `Runtime` to create a `System` future only.
    /// All the [`Arbiter`]s will be started in separate threads using their own tokio `Runtime`s.
    /// It means that using this method currently it is impossible to make `actix-rt` work in the
    /// alternative `tokio` `Runtime`s (e.g. provided by `tokio_compat`).
    ///
    /// [`tokio_compat`]: https://crates.io/crates/tokio-compat
    ///
    /// # Arguments
    ///
    /// - `name`: Name of the System
    /// - `runtime`: A tokio Runtime to run the system in.
    /// - `rest_operations`: A future to be executed in the runtime along with the System.
    ///
    /// # Examples
    ///
    /// ```
    /// use tokio::runtime::Runtime;
    /// use actix_rt::System;
    /// use futures_util::future::try_join_all;
    ///
    /// async fn run_application() {
    ///     let first_task = tokio::spawn(async {
    ///         // ...
    /// #        println!("One task");
    /// #        Ok::<(),()>(())
    ///     });
    ///
    ///     let second_task = tokio::spawn(async {
    ///         // ...
    /// #       println!("Another task");
    /// #       Ok::<(),()>(())
    ///     });
    ///
    ///     try_join_all(vec![first_task, second_task])
    ///         .await
    ///         .expect("Some of the futures finished unexpectedly");
    /// }
    ///
    ///
    /// let runtime = tokio::runtime::Builder::new()
    ///     .core_threads(2)
    ///     .enable_all()
    ///     .threaded_scheduler()
    ///     .build()
    ///     .unwrap();
    ///
    /// let rest_operations = run_application();
    /// System::attach_to_tokio("actix-main-system", runtime, rest_operations);
    /// ```
    pub fn attach_to_tokio<Fut, R>(
        name: impl Into<String>,
        mut runtime: tokio::runtime::Runtime,
        rest_operations: Fut,
    ) -> R
    where
        Fut: std::future::Future<Output = R>,
    {
        let actix_system_task = LocalSet::new();
        let sys = System::run_in_tokio(name.into(), &actix_system_task);
        actix_system_task.spawn_local(sys);

        runtime.block_on(actix_system_task.run_until(rest_operations))
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

    /// This function will start tokio runtime and will finish once the
    /// `System::stop()` message get called.
    /// Function `f` get called within tokio runtime context.
    pub fn run<F>(f: F) -> io::Result<()>
    where
        F: FnOnce() + 'static,
    {
        Self::builder().run(f)
    }
}
