use alloc::{rc::Rc, sync::Arc};
use core::{
    future::Future,
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

use futures_core::ready;
use pin_project_lite::pin_project;

use crate::{IntoServiceFactory, Service, ServiceFactory};

/// Apply a [`Transform`] to a [`Service`].
pub fn apply<T, S, I, Req>(t: T, factory: I) -> ApplyTransform<T, S, Req>
where
    I: IntoServiceFactory<S, Req>,
    S: ServiceFactory<Req>,
    T: Transform<S::Service, Req, InitError = S::InitError>,
{
    ApplyTransform::new(t, factory.into_factory())
}

/// Defines the interface of a service factory that wraps inner service during construction.
///
/// Transformers wrap an inner service and runs during inbound and/or outbound processing in the
/// service lifecycle. It may modify request and/or response.
///
/// For example, a timeout service wrapper:
///
/// ```ignore
/// pub struct Timeout<S> {
///     service: S,
///     timeout: Duration,
/// }
///
/// impl<S: Service<Req>, Req> Service<Req> for Timeout<S> {
///     type Response = S::Response;
///     type Error = TimeoutError<S::Error>;
///     type Future = TimeoutServiceResponse<S>;
///
///     actix_service::forward_ready!(service);
///
///     fn call(&self, req: Req) -> Self::Future {
///         TimeoutServiceResponse {
///             fut: self.service.call(req),
///             sleep: Sleep::new(clock::now() + self.timeout),
///         }
///     }
/// }
/// ```
///
/// This wrapper service is decoupled from the underlying service implementation and could be
/// applied to any service.
///
/// The `Transform` trait defines the interface of a service wrapper. `Transform` is often
/// implemented for middleware, defining how to construct a middleware Service. A Service that is
/// constructed by the factory takes the Service that follows it during execution as a parameter,
/// assuming ownership of the next Service.
///
/// A transform for the `Timeout` middleware could look like this:
///
/// ```ignore
/// pub struct TimeoutTransform {
///     timeout: Duration,
/// }
///
/// impl<S: Service<Req>, Req> Transform<S, Req> for TimeoutTransform {
///     type Response = S::Response;
///     type Error = TimeoutError<S::Error>;
///     type InitError = S::Error;
///     type Transform = Timeout<S>;
///     type Future = Ready<Result<Self::Transform, Self::InitError>>;
///
///     fn new_transform(&self, service: S) -> Self::Future {
///         ready(Ok(Timeout {
///             service,
///             timeout: self.timeout,
///         }))
///     }
/// }
/// ```
pub trait Transform<S, Req> {
    /// Responses produced by the service.
    type Response;

    /// Errors produced by the service.
    type Error;

    /// The `TransformService` value created by this factory
    type Transform: Service<Req, Response = Self::Response, Error = Self::Error>;

    /// Errors produced while building a transform service.
    type InitError;

    /// The future response value.
    type Future: Future<Output = Result<Self::Transform, Self::InitError>>;

    /// Creates and returns a new Transform component, asynchronously
    fn new_transform(&self, service: S) -> Self::Future;
}

impl<T, S, Req> Transform<S, Req> for Rc<T>
where
    T: Transform<S, Req>,
{
    type Response = T::Response;
    type Error = T::Error;
    type Transform = T::Transform;
    type InitError = T::InitError;
    type Future = T::Future;

    fn new_transform(&self, service: S) -> T::Future {
        self.as_ref().new_transform(service)
    }
}

impl<T, S, Req> Transform<S, Req> for Arc<T>
where
    T: Transform<S, Req>,
{
    type Response = T::Response;
    type Error = T::Error;
    type Transform = T::Transform;
    type InitError = T::InitError;
    type Future = T::Future;

    fn new_transform(&self, service: S) -> T::Future {
        self.as_ref().new_transform(service)
    }
}

/// Apply a [`Transform`] to a [`Service`].
pub struct ApplyTransform<T, S, Req>(Rc<(T, S)>, PhantomData<Req>);

impl<T, S, Req> ApplyTransform<T, S, Req>
where
    S: ServiceFactory<Req>,
    T: Transform<S::Service, Req, InitError = S::InitError>,
{
    /// Create new `ApplyTransform` new service instance
    fn new(t: T, service: S) -> Self {
        Self(Rc::new((t, service)), PhantomData)
    }
}

impl<T, S, Req> Clone for ApplyTransform<T, S, Req> {
    fn clone(&self) -> Self {
        ApplyTransform(self.0.clone(), PhantomData)
    }
}

impl<T, S, Req> ServiceFactory<Req> for ApplyTransform<T, S, Req>
where
    S: ServiceFactory<Req>,
    T: Transform<S::Service, Req, InitError = S::InitError>,
{
    type Response = T::Response;
    type Error = T::Error;

    type Config = S::Config;
    type Service = T::Transform;
    type InitError = T::InitError;
    type Future = ApplyTransformFuture<T, S, Req>;

    fn new_service(&self, cfg: S::Config) -> Self::Future {
        ApplyTransformFuture {
            store: self.0.clone(),
            state: ApplyTransformFutureState::A {
                fut: self.0.as_ref().1.new_service(cfg),
            },
        }
    }
}

pin_project! {
    pub struct ApplyTransformFuture<T, S, Req>
    where
        S: ServiceFactory<Req>,
        T: Transform<S::Service, Req, InitError = S::InitError>,
    {
        store: Rc<(T, S)>,
        #[pin]
        state: ApplyTransformFutureState<T, S, Req>,
    }
}

pin_project! {
    #[project = ApplyTransformFutureStateProj]
    pub enum ApplyTransformFutureState<T, S, Req>
    where
        S: ServiceFactory<Req>,
        T: Transform<S::Service, Req, InitError = S::InitError>,
    {
        A { #[pin] fut: S::Future },
        B { #[pin] fut: T::Future },
    }
}

impl<T, S, Req> Future for ApplyTransformFuture<T, S, Req>
where
    S: ServiceFactory<Req>,
    T: Transform<S::Service, Req, InitError = S::InitError>,
{
    type Output = Result<T::Transform, T::InitError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.as_mut().project();

        match this.state.as_mut().project() {
            ApplyTransformFutureStateProj::A { fut } => {
                let srv = ready!(fut.poll(cx))?;
                let fut = this.store.0.new_transform(srv);
                this.state.set(ApplyTransformFutureState::B { fut });
                self.poll(cx)
            }
            ApplyTransformFutureStateProj::B { fut } => fut.poll(cx),
        }
    }
}

#[cfg(test)]
mod tests {
    use core::time::Duration;

    use actix_utils::future::{ready, Ready};

    use super::*;
    use crate::Service;

    // pseudo-doctest for Transform trait
    pub struct TimeoutTransform {
        timeout: Duration,
    }

    // pseudo-doctest for Transform trait
    impl<S: Service<Req>, Req> Transform<S, Req> for TimeoutTransform {
        type Response = S::Response;
        type Error = S::Error;
        type InitError = S::Error;
        type Transform = Timeout<S>;
        type Future = Ready<Result<Self::Transform, Self::InitError>>;

        fn new_transform(&self, service: S) -> Self::Future {
            ready(Ok(Timeout {
                service,
                _timeout: self.timeout,
            }))
        }
    }

    // pseudo-doctest for Transform trait
    pub struct Timeout<S> {
        service: S,
        _timeout: Duration,
    }

    // pseudo-doctest for Transform trait
    impl<S: Service<Req>, Req> Service<Req> for Timeout<S> {
        type Response = S::Response;
        type Error = S::Error;
        type Future = S::Future;

        crate::forward_ready!(service);

        fn call(&self, req: Req) -> Self::Future {
            self.service.call(req)
        }
    }
}
