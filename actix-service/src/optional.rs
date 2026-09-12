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
