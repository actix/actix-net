//! Simple "poll function" future and factory.

use core::{
    fmt,
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

/// Creates a future driven by the provided function that receives a task context.
///
/// # Examples
/// ```
/// # use std::task::Poll;
/// # use actix_utils::future::poll_fn;
/// # async fn test_poll_fn() {
/// let res = poll_fn(|_| Poll::Ready(42)).await;
/// assert_eq!(res, 42);
///
/// let mut i = 5;
/// let res = poll_fn(|cx| {
///     i -= 1;
///
///     if i > 0 {
///         cx.waker().wake_by_ref();
///         Poll::Pending
///     } else {
///         Poll::Ready(42)
///     }
/// })
/// .await;
/// assert_eq!(res, 42);
/// # }
/// # actix_rt::Runtime::new().unwrap().block_on(test_poll_fn());
/// ```
#[inline]
pub fn poll_fn<F, T>(f: F) -> PollFn<F>
where
    F: FnMut(&mut Context<'_>) -> Poll<T>,
{
    PollFn { f }
}

/// Future for the [`poll_fn`] function.
pub struct PollFn<F> {
    f: F,
}

impl<F> fmt::Debug for PollFn<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PollFn").finish()
    }
}

impl<F, T> Future for PollFn<F>
where
    F: FnMut(&mut Context<'_>) -> Poll<T>,
{
    type Output = T;

    #[inline]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // SAFETY: we are not moving out of the pinned field
        // see https://github.com/rust-lang/rust/pull/102737
        (unsafe { &mut self.get_unchecked_mut().f })(cx)
    }
}

#[cfg(test)]
mod tests {
    use std::marker::PhantomPinned;

    use super::*;

    static_assertions::assert_impl_all!(PollFn<()>: Unpin);
    static_assertions::assert_not_impl_all!(PollFn<PhantomPinned>: Unpin);

    #[actix_rt::test]
    async fn test_poll_fn() {
        let res = poll_fn(|_| Poll::Ready(42)).await;
        assert_eq!(res, 42);

        let mut i = 5;
        let res = poll_fn(|cx| {
            i -= 1;

            if i > 0 {
                cx.waker().wake_by_ref();
                Poll::Pending
            } else {
                Poll::Ready(42)
            }
        })
        .await;
        assert_eq!(res, 42);
    }

    // following soundness tests taken from https://github.com/tokio-rs/tokio/pull/5087

    #[allow(dead_code)]
    fn require_send<T: Send>(_t: &T) {}
    #[allow(dead_code)]
    fn require_sync<T: Sync>(_t: &T) {}

    trait AmbiguousIfUnpin<A> {
        fn some_item(&self) {}
    }
    impl<T: ?Sized> AmbiguousIfUnpin<()> for T {}
    impl<T: ?Sized + Unpin> AmbiguousIfUnpin<[u8; 0]> for T {}

    const _: fn() = || {
        let pinned = std::marker::PhantomPinned;
        let f = poll_fn(move |_| {
            // Use `pinned` to take ownership of it.
            let _ = &pinned;
            std::task::Poll::Pending::<()>
        });
        require_send(&f);
        require_sync(&f);
        AmbiguousIfUnpin::some_item(&f);
    };
}
