//! Thread pool for blocking operations

use std::future::Future;
use std::task::{Poll,Context};

use derive_more::Display;
use futures::channel::oneshot;
use parking_lot::Mutex;
use threadpool::ThreadPool;
use std::pin::Pin;

/// Env variable for default cpu pool size
const ENV_CPU_POOL_VAR: &str = "ACTIX_THREADPOOL";

lazy_static::lazy_static! {
    pub(crate) static ref DEFAULT_POOL: Mutex<ThreadPool> = {
        let default = match std::env::var(ENV_CPU_POOL_VAR) {
            Ok(val) => {
                if let Ok(val) = val.parse() {
                    val
                } else {
                    log::error!("Can not parse ACTIX_THREADPOOL value");
                    num_cpus::get() * 5
                }
            }
            Err(_) => num_cpus::get() * 5,
        };
        Mutex::new(
            threadpool::Builder::new()
                .thread_name("actix-web".to_owned())
                .num_threads(default)
                .build(),
        )
    };
}

thread_local! {
    static POOL: ThreadPool = {
        DEFAULT_POOL.lock().clone()
    };
}

/// Blocking operation execution error
#[derive(Debug, Display)]
#[display(fmt = "Thread pool is gone")]
pub struct Cancelled;

/// Execute blocking function on a thread pool, returns future that resolves
/// to result of the function execution.
pub fn run<F, I>(f: F) -> CpuFuture<I>
where
    F: FnOnce() -> I + Send + 'static,
    I: Send + 'static,
{
    let (tx, rx) = oneshot::channel();
    POOL.with(|pool| {
        pool.execute(move || {
            if !tx.is_canceled() {
                let _ = tx.send(f());
            }
        })
    });

    CpuFuture { rx }
}

/// Blocking operation completion future. It resolves with results
/// of blocking function execution.
pub struct CpuFuture<I> {
    rx: oneshot::Receiver<I>,
}

impl<I> Future for CpuFuture<I> {
    type Output = Result<I,Cancelled>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let rx  = Pin::new(&mut Pin::get_mut(self).rx);
        let res = futures::ready!(rx.poll(cx));
        Poll::Ready(res.map_err(|_| Cancelled))
    }

}
