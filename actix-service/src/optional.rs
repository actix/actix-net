use core::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use futures_core::ready;
use pin_project_lite::pin_project;

use crate::{Service, Transform};

/// Applies the transform when present, or uses the inner service unchanged.
///
/// The transform must preserve the inner service's response and error types.
impl<T, S, Req> Transform<S, Req> for Option<T>
where
    S: Service<Req>,
    T: Transform<S, Req, Response = S::Response, Error = S::Error>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Transform = OptionalService<T::Transform, S>;
    type InitError = T::InitError;
    type Future = OptionalTransformFuture<T::Future, S>;

    fn new_transform(&self, service: S) -> Self::Future {
        match self {
            Some(transform) => OptionalTransformFuture::Enabled {
                future: transform.new_transform(service),
            },
            None => OptionalTransformFuture::Disabled {
                service: Some(service),
            },
        }
    }
}

/// A service created by an optional transform.
pub enum OptionalService<T, S> {
    /// The transform is enabled.
    Enabled(T),
    /// The inner service is used directly.
    Disabled(S),
}

impl<T, S, Req> Service<Req> for OptionalService<T, S>
where
    S: Service<Req>,
    T: Service<Req, Response = S::Response, Error = S::Error>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = OptionalServiceFuture<T::Future, S::Future>;

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        match self {
            Self::Enabled(service) => service.poll_ready(cx),
            Self::Disabled(service) => service.poll_ready(cx),
        }
    }

    fn call(&self, req: Req) -> Self::Future {
        match self {
            Self::Enabled(service) => OptionalServiceFuture::Enabled {
                future: service.call(req),
            },
            Self::Disabled(service) => OptionalServiceFuture::Disabled {
                future: service.call(req),
            },
        }
    }
}

pin_project! {
    /// Future returned when constructing an optional transform.
    #[doc(hidden)]
    #[project = OptionalTransformFutureProj]
    pub enum OptionalTransformFuture<F, S> {
        /// Constructs the middleware service.
        Enabled {
            #[pin]
            future: F
        },
        /// Returns the inner service.
        Disabled {
            service: Option<S>
        },
    }
}

impl<F, S, T, E> Future for OptionalTransformFuture<F, S>
where
    F: Future<Output = Result<T, E>>,
{
    type Output = Result<OptionalService<T, S>, E>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.project() {
            OptionalTransformFutureProj::Enabled { future } => {
                Poll::Ready(Ok(OptionalService::Enabled(ready!(future.poll(cx))?)))
            }
            OptionalTransformFutureProj::Disabled { service } => Poll::Ready(Ok(
                OptionalService::Disabled(service.take().expect("future polled after completion")),
            )),
        }
    }
}

pin_project! {
    /// Future returned by an optional transform's service.
    #[doc(hidden)]
    #[project = OptionalServiceFutureProj]
    pub enum OptionalServiceFuture<T, S> {
        /// Runs the middleware service.
        Enabled {
            #[pin]
            future: T
        },
        /// Runs the inner service directly.
        Disabled {
            #[pin]
            future: S
        },
    }
}

impl<T, S> Future for OptionalServiceFuture<T, S>
where
    T: Future,
    S: Future<Output = T::Output>,
{
    type Output = T::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.project() {
            OptionalServiceFutureProj::Enabled { future } => future.poll(cx),
            OptionalServiceFutureProj::Disabled { future } => future.poll(cx),
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::boxed::Box;
    use core::{
        future::{poll_fn, Future},
        pin::Pin,
        task::{Context, Poll},
    };

    use crate::{fn_service, Service, Transform};

    struct AddOne(bool);

    impl<S> Transform<S, u32> for AddOne
    where
        S: Service<u32, Response = u32, Error = &'static str> + 'static,
    {
        type Response = u32;
        type Error = &'static str;
        type Transform = Wrapped<S>;
        type InitError = &'static str;
        type Future = Pin<Box<dyn Future<Output = Result<Self::Transform, Self::InitError>>>>;

        fn new_transform(&self, service: S) -> Self::Future {
            let fail = self.0;
            Box::pin(async move {
                let mut pending = true;
                poll_fn(move |cx| {
                    if core::mem::take(&mut pending) {
                        cx.waker().wake_by_ref();
                        Poll::Pending
                    } else {
                        Poll::Ready(())
                    }
                })
                .await;
                if fail {
                    Err("init")
                } else {
                    Ok(Wrapped(service))
                }
            })
        }
    }

    struct Wrapped<S>(S);

    impl<S> Service<u32> for Wrapped<S>
    where
        S: Service<u32, Response = u32, Error = &'static str>,
    {
        type Response = u32;
        type Error = &'static str;
        type Future = S::Future;

        fn poll_ready(&self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Err("middleware readiness"))
        }

        fn call(&self, req: u32) -> Self::Future {
            self.0.call(req + 1)
        }
    }

    #[actix_rt::test]
    async fn optional_transform() {
        for enabled in [false, true] {
            let service = enabled
                .then_some(AddOne(false))
                .new_transform(fn_service(|req: u32| async move {
                    if req > 10 {
                        Err("call")
                    } else {
                        Ok(req)
                    }
                }))
                .await
                .unwrap();

            assert_eq!(service.call(1).await, Ok(if enabled { 2 } else { 1 }));
            assert_eq!(service.call(11).await, Err("call"));
            assert_eq!(
                poll_fn(|cx| service.poll_ready(cx)).await,
                if enabled {
                    Err("middleware readiness")
                } else {
                    Ok(())
                }
            );
        }
    }

    #[actix_rt::test]
    async fn optional_transform_init_error() {
        let result = Some(AddOne(true))
            .new_transform(fn_service(|req: u32| async move { Ok(req) }))
            .await;
        assert!(matches!(result, Err("init")));
    }

    struct PendingService(core::cell::Cell<bool>);

    impl Service<u32> for PendingService {
        type Response = u32;
        type Error = &'static str;
        type Future = Pin<Box<dyn Future<Output = Result<u32, Self::Error>>>>;

        fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            if self.0.replace(false) {
                cx.waker().wake_by_ref();
                Poll::Pending
            } else {
                Poll::Ready(Err("inner readiness"))
            }
        }

        fn call(&self, req: u32) -> Self::Future {
            let mut pending = true;
            Box::pin(poll_fn(move |cx| {
                if core::mem::take(&mut pending) {
                    cx.waker().wake_by_ref();
                    Poll::Pending
                } else {
                    Poll::Ready(Ok(req))
                }
            }))
        }
    }

    #[actix_rt::test]
    async fn optional_transform_pending() {
        for enabled in [false, true] {
            let mut constructed = false;
            let service = enabled
                .then(|| {
                    constructed = true;
                    AddOne(false)
                })
                .new_transform(PendingService(core::cell::Cell::new(true)))
                .await
                .unwrap();
            assert_eq!(constructed, enabled);
            let mut cx = Context::from_waker(futures_util::task::noop_waker_ref());
            if !enabled {
                assert_eq!(service.poll_ready(&mut cx), Poll::Pending);
                assert_eq!(
                    service.poll_ready(&mut cx),
                    Poll::Ready(Err("inner readiness"))
                );
            }
            let future = service.call(5);
            let mut future = core::pin::pin!(future);
            assert_eq!(future.as_mut().poll(&mut cx), Poll::Pending);
            assert_eq!(future.await, Ok(if enabled { 6 } else { 5 }));
        }
    }
}
