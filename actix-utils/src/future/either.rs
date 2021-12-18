//! A symmetric either future.

use core::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use pin_project_lite::pin_project;

pin_project! {
    /// Combines two different futures that have the same output type.
    ///
    /// Construct variants with [`Either::left`] and [`Either::right`].
    ///
    /// # Examples
    /// ```
    /// use actix_utils::future::{ready, Ready, Either};
    ///
    /// # async fn run() {
    /// let res = Either::<_, Ready<usize>>::left(ready(42));
    /// assert_eq!(res.await, 42);
    ///
    /// let res = Either::<Ready<usize>, _>::right(ready(43));
    /// assert_eq!(res.await, 43);
    /// # }
    /// ```
    #[project = EitherProj]
    #[derive(Debug, Clone)]
    pub enum Either<L, R> {
        /// A value of type `L`.
        #[allow(missing_docs)]
        Left { #[pin] value: L },

        /// A value of type `R`.
        #[allow(missing_docs)]
        Right { #[pin] value: R },
    }
}

impl<L, R> Either<L, R> {
    /// Creates new `Either` using left variant.
    #[inline]
    pub fn left(value: L) -> Either<L, R> {
        Either::Left { value }
    }

    /// Creates new `Either` using right variant.
    #[inline]
    pub fn right(value: R) -> Either<L, R> {
        Either::Right { value }
    }
}

impl<T> Either<T, T> {
    /// Unwraps into inner value when left and right have a common type.
    #[inline]
    pub fn into_inner(self) -> T {
        match self {
            Either::Left { value } => value,
            Either::Right { value } => value,
        }
    }
}

impl<L, R> Future for Either<L, R>
where
    L: Future,
    R: Future<Output = L::Output>,
{
    type Output = L::Output;

    #[inline]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.project() {
            EitherProj::Left { value } => value.poll(cx),
            EitherProj::Right { value } => value.poll(cx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::future::{ready, Ready};

    #[actix_rt::test]
    async fn test_either() {
        let res = Either::<_, Ready<usize>>::left(ready(42));
        assert_eq!(res.await, 42);

        let res = Either::<Ready<usize>, _>::right(ready(43));
        assert_eq!(res.await, 43);
    }
}
