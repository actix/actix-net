use std::{future::Future, io};

use tokio::task::{JoinHandle, LocalSet};

/// Single-threaded runtime provides a way to start reactor and runtime on the current thread.
///
/// See [crate root][crate] documentation for more details.
#[derive(Debug)]
pub struct Runtime {
    local: LocalSet,
    rt: tokio::runtime::Runtime,
}

impl Runtime {
    /// Returns a new runtime initialized with default configuration values.
    #[allow(clippy::new_ret_no_self)]
    pub fn new() -> io::Result<Runtime> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()?;

        Ok(Runtime {
            rt,
            local: LocalSet::new(),
        })
    }

    /// Reference to local task set.
    pub(crate) fn local(&self) -> &LocalSet {
        &self.local
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
    pub fn block_on<F>(&self, f: F) -> F::Output
    where
        F: Future,
    {
        self.local.block_on(&self.rt, f)
    }
}
