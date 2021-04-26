//! Simple "poll function" future and factory.

use core::{
    fmt,
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

/// Creates a future driven by the provided function that receives a task context.
pub fn poll_fn<F, T>(f: F) -> PollFn<F>
where
    F: FnMut(&mut Context<'_>) -> Poll<T>,
{
    PollFn { f }
}

/// A Future driven by the inner function.
pub struct PollFn<F> {
    f: F,
}

impl<F> Unpin for PollFn<F> {}

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

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        (self.f)(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
