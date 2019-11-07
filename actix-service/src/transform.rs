use std::rc::Rc;
use std::sync::Arc;

use crate::transform_err::{TransformFromErr, TransformMapInitErr};
use crate::{IntoNewService, NewService, Service};
use futures::{Future, Poll};
use std::pin::Pin;
use std::task::Context;

use pin_project::pin_project;

/// The `Transform` trait defines the interface of a Service factory. `Transform`
/// is often implemented for middleware, defining how to construct a
/// middleware Service. A Service that is constructed by the factory takes
/// the Service that follows it during execution as a parameter, assuming
/// ownership of the next Service.
pub trait Transform<S> {
    /// Requests handled by the service.
    type Request;

    /// Responses given by the service.
    type Response;

    /// Errors produced by the service.
    type Error;

    /// The `TransformService` value created by this factory
    type Transform: Service<
        Request = Self::Request,
        Response = Self::Response,
        Error = Self::Error,
    >;

    /// Errors produced while building a service.
    type InitError;

    /// The future response value.
    type Future: Future<Output = Result<Self::Transform, Self::InitError>>;

    /// Creates and returns a new Service component, asynchronously
    fn new_transform(&self, service: S) -> Self::Future;

    /// Map this service's factory error to a different error,
    /// returning a new transform service factory.
    fn map_init_err<F, E>(self, f: F) -> TransformMapInitErr<Self, S, F, E>
    where
        Self: Sized,
        F: Fn(Self::InitError) -> E,
    {
        TransformMapInitErr::new(self, f)
    }

    /// Map this service's init error to any error implementing `From` for
    /// this service`s `Error`.
    ///
    /// Note that this function consumes the receiving transform and returns a
    /// wrapped version of it.
    fn from_err<E>(self) -> TransformFromErr<Self, S, E>
    where
        Self: Sized,
        E: From<Self::InitError>,
    {
        TransformFromErr::new(self)
    }

    // /// Map this service's init error to service's init error
    // /// if it is implementing `Into` to this service`s `InitError`.
    // ///
    // /// Note that this function consumes the receiving transform and returns a
    // /// wrapped version of it.
    // fn into_err<E>(self) -> TransformIntoErr<Self, S>
    // where
    //     Self: Sized,
    //     Self::InitError: From<Self::InitError>,
    // {
    //     TransformFromErr::new(self)
    // }
}

impl<T, S> Transform<S> for Rc<T>
where
    T: Transform<S>,
{
    type Request = T::Request;
    type Response = T::Response;
    type Error = T::Error;
    type InitError = T::InitError;
    type Transform = T::Transform;
    type Future = T::Future;

    fn new_transform(&self, service: S) -> T::Future {
        self.as_ref().new_transform(service)
    }
}

impl<T, S> Transform<S> for Arc<T>
where
    T: Transform<S>,
{
    type Request = T::Request;
    type Response = T::Response;
    type Error = T::Error;
    type InitError = T::InitError;
    type Transform = T::Transform;
    type Future = T::Future;

    fn new_transform(&self, service: S) -> T::Future {
        self.as_ref().new_transform(service)
    }
}

/// Trait for types that can be converted to a *transform service*
pub trait IntoTransform<T, S>
where
    T: Transform<S>,
{
    /// Convert to a `TransformService`
    fn into_transform(self) -> T;
}

impl<T, S> IntoTransform<T, S> for T
where
    T: Transform<S>,
{
    fn into_transform(self) -> T {
        self
    }
}

/// Apply transform to service factory. Function returns
/// services factory that in initialization creates
/// service and applies transform to this service.
pub fn apply_transform<T, S, F, U>(
    t: F,
    service: U,
) -> impl NewService<
    Config = S::Config,
    Request = T::Request,
    Response = T::Response,
    Error = T::Error,
    Service = T::Transform,
    InitError = S::InitError,
> + Clone
where
    S: NewService,
    T: Transform<S::Service, InitError = S::InitError>,
    F: IntoTransform<T, S::Service>,
    U: IntoNewService<S>,
{
    ApplyTransform::new(t.into_transform(), service.into_new_service())
}

/// `Apply` transform to new service
pub struct ApplyTransform<T, S> {
    s: Rc<S>,
    t: Rc<T>,
}

impl<T, S> ApplyTransform<T, S>
where
    S: NewService,
    T: Transform<S::Service, InitError = S::InitError>,
{
    /// Create new `ApplyTransform` new service instance
    pub fn new<F: IntoTransform<T, S::Service>>(t: F, service: S) -> Self {
        Self {
            s: Rc::new(service),
            t: Rc::new(t.into_transform()),
        }
    }
}

impl<T, S> Clone for ApplyTransform<T, S> {
    fn clone(&self) -> Self {
        ApplyTransform {
            s: self.s.clone(),
            t: self.t.clone(),
        }
    }
}

impl<T, S> NewService for ApplyTransform<T, S>
where
    S: NewService,
    T: Transform<S::Service, InitError = S::InitError>,
{
    type Request = T::Request;
    type Response = T::Response;
    type Error = T::Error;

    type Config = S::Config;
    type Service = T::Transform;
    type InitError = T::InitError;
    type Future = ApplyTransformFuture<T, S>;

    fn new_service(&self, cfg: &S::Config) -> Self::Future {
        ApplyTransformFuture {
            t_cell: self.t.clone(),
            fut_a: self.s.new_service(cfg),
            fut_t: None,
        }
    }
}
#[pin_project]
pub struct ApplyTransformFuture<T, S>
where
    S: NewService,
    T: Transform<S::Service, InitError = S::InitError>,
{
    #[pin]
    fut_a: S::Future,
    #[pin]
    fut_t: Option<T::Future>,
    t_cell: Rc<T>,
}

impl<T, S> Future for ApplyTransformFuture<T, S>
where
    S: NewService,
    T: Transform<S::Service, InitError = S::InitError>,
{
    type Output = Result<T::Transform, T::InitError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();

        if this.fut_t.as_mut().as_pin_mut().is_none() {
            if let Poll::Ready(service) = this.fut_a.poll(cx)? {
                this.fut_t.set(Some(this.t_cell.new_transform(service)));
            }
        }

        if let Some(fut) = this.fut_t.as_mut().as_pin_mut() {
            fut.poll(cx)
        } else {
            Poll::Pending
        }
    }
}
