use std::future::Future;
use std::io;
use std::time::Duration;

use tokio::{runtime, task::LocalSet};

/// A trait for construct async executor and run future on it.
///
/// A factory trait is necessary as `actix` and `actix-web` can run on multiple instances of
/// executors. Therefore the executor would be constructed multiple times
pub trait ExecFactory: Sized + Send + Sync + Unpin + 'static {
    type Executor;
    type Sleep: Future<Output = ()> + Send + Unpin + 'static;

    fn build() -> io::Result<Self::Executor>;

    /// Runs the provided future, blocking the current thread until the future
    /// completes.
    ///
    /// This function can be used to synchronously block the current thread
    /// until the provided `future` has resolved either successfully or with an
    /// error. The result of the future is then returned from this function
    /// call.
    ///
    /// Note that this function will **also** execute any spawned futures on the
    /// current thread, but will **not** block until these other spawned futures
    /// have completed. Once the function returns, any uncompleted futures
    /// remain pending in the `Runtime` instance. These futures will not run
    /// until `block_on` or `run` is called again.
    ///
    /// The caller is responsible for ensuring that other spawned futures
    /// complete execution by calling `block_on` or `run`.
    fn block_on<F: Future>(exec: &mut Self::Executor, f: F) -> F::Output;

    /// Spawn a future onto the single-threaded runtime without reference it.
    ///
    /// See [module level][mod] documentation for more details.
    ///
    /// [mod]: index.html
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// # use futures::{future, Future, Stream};
    /// use actix_rt::Runtime;
    ///
    /// # fn dox() {
    /// // Create the runtime
    /// let mut rt = Runtime::new().unwrap();
    ///
    /// // Spawn a future onto the runtime
    /// rt.spawn(future::lazy(|_| {
    ///     println!("running on the runtime");
    /// }));
    /// # }
    /// # pub fn main() {}
    /// ```
    ///
    /// # Panics
    ///
    /// This function panics if the spawn fails. Failure occurs if the executor
    /// is currently at capacity and is unable to spawn a new future.
    fn spawn<F: Future<Output = ()> + 'static>(f: F);

    /// Spawn a future onto the single-threaded runtime reference. Useful when you have direct
    /// access to executor.
    ///
    /// *. `spawn_ref` is preferred when you can choose between it and `spawn`.
    fn spawn_ref<F: Future<Output = ()> + 'static>(exec: &mut Self::Executor, f: F);

    /// Get a timeout sleep future with given duration.
    fn sleep(dur: Duration) -> Self::Sleep;
}

/// Default Single-threaded tokio executor on the current thread.
///
/// See [module level][mod] documentation for more details.
///
/// [mod]: index.html
#[derive(Copy, Clone, Debug)]
pub struct ActixExec;

impl ExecFactory for ActixExec {
    type Executor = (runtime::Runtime, LocalSet);
    type Sleep = tokio::time::Delay;

    fn build() -> io::Result<Self::Executor> {
        let rt = runtime::Builder::new()
            .enable_io()
            .enable_time()
            .basic_scheduler()
            .build()?;

        Ok((rt, LocalSet::new()))
    }

    fn block_on<F: Future>(exec: &mut Self::Executor, f: F) -> <F as Future>::Output {
        let (rt, local) = exec;

        rt.block_on(local.run_until(f))
    }

    #[inline]
    fn spawn<F: Future<Output = ()> + 'static>(f: F) {
        tokio::task::spawn_local(f);
    }

    fn spawn_ref<F: Future<Output = ()> + 'static>(exec: &mut Self::Executor, f: F) {
        exec.1.spawn_local(f);
    }

    #[inline]
    fn sleep(dur: Duration) -> Self::Sleep {
        tokio::time::delay_for(dur)
    }
}
