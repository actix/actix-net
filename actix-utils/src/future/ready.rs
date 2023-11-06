//! When `core::future::Ready` has a `into_inner()` method, this can be deprecated.

use core::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

/// Future for the [`ready`] function.
///
/// Panic will occur if polled more than once.
///
/// # Examples
/// ```
/// use actix_utils::future::ready;
///
/// // async
/// # async fn run() {
/// let a = ready(1);
/// assert_eq!(a.await, 1);
/// # }
///
/// // sync
/// let a = ready(1);
/// assert_eq!(a.into_inner(), 1);
/// ```
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
        let val = self.val.take().expect("Ready polled after completion");
        Poll::Ready(val)
    }
}

/// Creates a future that is immediately ready with a value.
///
/// # Examples
/// ```no_run
/// use actix_utils::future::ready;
///
/// # async fn run() {
/// let a = ready(1);
/// assert_eq!(a.await, 1);
/// # }
///
/// // sync
/// let a = ready(1);
/// assert_eq!(a.into_inner(), 1);
/// ```
#[inline]
pub fn ready<T>(val: T) -> Ready<T> {
    Ready { val: Some(val) }
}

/// Creates a future that is immediately ready with a success value.
///
/// # Examples
/// ```no_run
/// use actix_utils::future::ok;
///
/// # async fn run() {
/// let a = ok::<_, ()>(1);
/// assert_eq!(a.await, Ok(1));
/// # }
/// ```
#[inline]
pub fn ok<T, E>(val: T) -> Ready<Result<T, E>> {
    Ready { val: Some(Ok(val)) }
}

/// Creates a future that is immediately ready with an error value.
///
/// # Examples
/// ```no_run
/// use actix_utils::future::err;
///
/// # async fn run() {
/// let a = err::<(), _>(1);
/// assert_eq!(a.await, Err(1));
/// # }
/// ```
#[inline]
pub fn err<T, E>(err: E) -> Ready<Result<T, E>> {
    Ready {
        val: Some(Err(err)),
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use futures_util::task::noop_waker;
    use static_assertions::{assert_impl_all, assert_not_impl_any};

    use super::*;

    assert_impl_all!(Ready<()>: Send, Sync, Unpin, Clone);
    assert_impl_all!(Ready<Rc<()>>: Unpin, Clone);
    assert_not_impl_any!(Ready<Rc<()>>: Send, Sync);

    #[test]
    #[should_panic]
    fn multiple_poll_panics() {
        let waker = noop_waker();
        let mut cx = Context::from_waker(&waker);

        let mut ready = ready(1);
        assert_eq!(Pin::new(&mut ready).poll(&mut cx), Poll::Ready(1));

        // panic!
        let _ = Pin::new(&mut ready).poll(&mut cx);
    }
}
