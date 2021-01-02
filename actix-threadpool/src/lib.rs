//! Thread pool for blocking operations

#![deny(rust_2018_idioms, nonstandard_style)]
#![doc(html_logo_url = "https://actix.rs/img/logo.png")]
#![doc(html_favicon_url = "https://actix.rs/favicon.ico")]

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::task::JoinHandle;

/// Blocking operation execution error
#[derive(Debug)]
pub enum BlockingError<E: fmt::Debug> {
    Error(E),
    Canceled,
}

impl<E: fmt::Debug> fmt::Display for BlockingError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Error(ref e) => write!(f, "{:?}", e),
            Self::Canceled => write!(f, "Thread pool is gone"),
        }
    }
}

impl<E: fmt::Debug> std::error::Error for BlockingError<E> {}

/// Execute blocking function on a thread pool, returns future that resolves
/// to result of the function execution.
pub fn run<F, I, E>(f: F) -> CpuFuture<I, E>
where
    F: FnOnce() -> Result<I, E> + Send + 'static,
    I: Send + 'static,
    E: Send + fmt::Debug + 'static,
{
    let handle = tokio::task::spawn_blocking(f);

    CpuFuture { handle }
}

/// Blocking operation completion future. It resolves with results
/// of blocking function execution.
pub struct CpuFuture<I, E> {
    handle: JoinHandle<Result<I, E>>,
}

impl<I, E: fmt::Debug> Future for CpuFuture<I, E> {
    type Output = Result<I, BlockingError<E>>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match Pin::new(&mut self.handle).poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Ok(res)) => Poll::Ready(res.map_err(BlockingError::Error)),
            Poll::Ready(Err(_)) => Poll::Ready(Err(BlockingError::Canceled)),
        }
    }
}
