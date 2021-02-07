//! When MSRV is 1.48, replace with `core::future::Ready` and `core::future::ready()`.

use core::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

/// Future for the [`ready`](ready()) function.
#[derive(Debug, Clone)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct Ready<T> {
    val: Option<T>,
}

impl<T> Ready<T> {
    /// Unwraps the value from this immediately ready future.
    #[inline]
    pub fn into_inner(mut self) -> T {
        self.val.take().unwrap()
    }
}

impl<T> Unpin for Ready<T> {}

impl<T> Future for Ready<T> {
    type Output = T;

    #[inline]
    fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<T> {
        let val = self.val.take().expect("Ready can not be polled twice.");
        Poll::Ready(val)
    }
}

/// Creates a future that is immediately ready with a value.
#[allow(dead_code)]
pub(crate) fn ready<T>(val: T) -> Ready<T> {
    Ready { val: Some(val) }
}

/// Create a future that is immediately ready with a success value.
#[allow(dead_code)]
pub(crate) fn ok<T, E>(val: T) -> Ready<Result<T, E>> {
    Ready { val: Some(Ok(val)) }
}

/// Create a future that is immediately ready with an error value.
#[allow(dead_code)]
pub(crate) fn err<T, E>(err: E) -> Ready<Result<T, E>> {
    Ready {
        val: Some(Err(err)),
    }
}
