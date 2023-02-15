use std::{future::Future, io};

use tokio::task::{JoinHandle, LocalSet};

/// A Tokio-based runtime proxy.
///
/// All spawned futures will be executed on the current thread. Therefore, there is no `Send` bound
/// on submitted futures.
#[cfg_attr(not(all(target_os = "linux", feature = "io-uring")), derive(Debug))]
pub struct Runtime {
    local: LocalSet,
    rt: GlobalRuntime,
}

#[cfg(all(target_os = "linux", feature = "io-uring"))]
impl std::fmt::Debug for Runtime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Runtime")
            .field("local", &self.local)
            .finish_non_exhaustive()
    }
}

#[cfg(all(target_os = "linux", feature = "io-uring"))]
pub(crate) type GlobalRuntime = tokio_uring::Runtime;

#[cfg(not(all(target_os = "linux", feature = "io-uring")))]
pub(crate) type GlobalRuntime = tokio::runtime::Runtime;

#[cfg(not(all(target_os = "linux", feature = "io-uring")))]
pub(crate) fn default_tokio_runtime() -> io::Result<GlobalRuntime> {
    tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
}

#[cfg(all(target_os = "linux", feature = "io-uring"))]
pub(crate) fn default_tokio_runtime() -> io::Result<GlobalRuntime> {
    tokio_uring::Runtime::new(&tokio_uring::builder())
}

impl Runtime {
    /// Returns a new runtime initialized with default configuration values.
    #[allow(clippy::new_ret_no_self)]
    pub fn new() -> io::Result<Self> {
        let rt = default_tokio_runtime()?;

        Ok(Runtime {
            rt,
            local: LocalSet::new(),
        })
    }

    /// Offload a future onto the single-threaded runtime.
    ///
    /// The returned join handle can be used to await the future's result.
    ///
    /// See [crate root][crate] documentation for more details.
    ///
    /// # Examples
    /// ```
    /// let rt = actix_rt::Runtime::new().unwrap();
    ///
    /// // Spawn a future onto the runtime
    /// let handle = rt.spawn(async {
    ///     println!("running on the runtime");
    ///     42
    /// });
    ///
    /// assert_eq!(rt.block_on(handle).unwrap(), 42);
    /// ```
    ///
    /// # Panics
    /// This function panics if the spawn fails. Failure occurs if the executor is currently at
    /// capacity and is unable to spawn a new future.
    #[track_caller]
    pub fn spawn<F>(&self, future: F) -> JoinHandle<F::Output>
    where
        F: Future + 'static,
    {
        self.local.spawn_local(future)
    }

    /// Runs the provided future, blocking the current thread until the future completes.
    ///
    /// This function can be used to synchronously block the current thread until the provided
    /// `future` has resolved either successfully or with an error. The result of the future is
    /// then returned from this function call.
    ///
    /// Note that this function will also execute any spawned futures on the current thread, but
    /// will not block until these other spawned futures have completed. Once the function returns,
    /// any uncompleted futures remain pending in the `Runtime` instance. These futures will not run
    /// until `block_on` or `run` is called again.
    ///
    /// The caller is responsible for ensuring that other spawned futures complete execution by
    /// calling `block_on` or `run`.
    #[track_caller]
    pub fn block_on<F>(&self, f: F) -> F::Output
    where
        F: Future,
    {
        self.rt.block_on(self.local.run_until(f))
    }
}

impl From<GlobalRuntime> for Runtime {
    fn from(rt: GlobalRuntime) -> Self {
        Self {
            local: LocalSet::new(),
            rt,
        }
    }
}
