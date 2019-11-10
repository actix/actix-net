use std::cell::RefCell;
use std::future::Future;
use std::rc::Rc;
use std::sync::Arc;
use std::task::{self, Context, Poll};

mod and_then;
mod apply;
mod apply_cfg;
pub mod boxed;
mod cell;
mod fn_service;
mod map;
mod map_config;
mod map_err;
mod map_init_err;
mod pipeline;
mod then;
mod transform;
mod transform_err;

pub use self::apply::{apply_fn, new_apply_fn};
pub use self::apply_cfg::{apply_cfg, new_apply_cfg};
pub use self::fn_service::{new_service_cfg, new_service_fn, service_fn, ServiceFn};
pub use self::map::{Map, MapNewService};
pub use self::map_config::{MapConfig, MappedConfig, UnitConfig};
pub use self::map_err::{MapErr, MapErrNewService};
pub use self::map_init_err::MapInitErr;
pub use self::pipeline::{new_pipeline, pipeline, NewPipeline, Pipeline};
pub use self::then::{Then, ThenNewService};
pub use self::transform::{apply_transform, IntoTransform, Transform};

pub trait IntoFuture {
    type Item;
    type Error;
    type Future: Future<Output = Result<Self::Item, Self::Error>>;
    fn into_future(self) -> Self::Future;
}

impl<F: Future<Output = Result<I, E>>, I, E> IntoFuture for F {
    type Item = I;
    type Error = E;
    type Future = F;

    fn into_future(self) -> Self::Future {
        self
    }
}

pub fn service<T, U>(factory: U) -> T
where
    T: Service,
    U: IntoService<T>,
{
    factory.into_service()
}

pub fn new_service<T, U>(factory: U) -> T
where
    T: NewService,
    U: IntoNewService<T>,
{
    factory.into_new_service()
}

/// An asynchronous function from `Request` to a `Response`.
pub trait Service {
    /// Requests handled by the service.
    type Request;

    /// Responses given by the service.
    type Response;

    /// Errors produced by the service.
    type Error;

    /// The future response value.
    type Future: Future<Output = Result<Self::Response, Self::Error>>;

    /// Returns `Ready` when the service is able to process requests.
    ///
    /// If the service is at capacity, then `NotReady` is returned and the task
    /// is notified when the service becomes ready again. This function is
    /// expected to be called while on a task.
    ///
    /// This is a **best effort** implementation. False positives are permitted.
    /// It is permitted for the service to return `Ready` from a `poll_ready`
    /// call and the next invocation of `call` results in an error.
    fn poll_ready(&mut self, ctx: &mut task::Context<'_>) -> Poll<Result<(), Self::Error>>;

    /// Process the request and return the response asynchronously.
    ///
    /// This function is expected to be callable off task. As such,
    /// implementations should take care to not call `poll_ready`. If the
    /// service is at capacity and the request is unable to be handled, the
    /// returned `Future` should resolve to an error.
    ///
    /// Calling `call` without calling `poll_ready` is permitted. The
    /// implementation must be resilient to this fact.
    fn call(&mut self, req: Self::Request) -> Self::Future;

    // #[cfg(test)]
    // fn poll_test(&mut self) -> Poll<Result<(), Self::Error>> {
    //     // kinda stupid method, but works for our test purposes
    //     unsafe {
    //         let mut this = Pin::new_unchecked(self);
    //         tokio::runtime::current_thread::Builder::new()
    //             .build()
    //             .unwrap()
    //             .block_on(futures::future::poll_fn(move |cx| {
    //                 let this = &mut this;
    //                 Poll::Ready(this.as_mut().poll_ready(cx))
    //             }))
    //     }
    // }

    // fn poll_once<'a>(&'a mut self) -> LocalBoxFuture<'a, Poll<Result<(), Self::Error>>> {
    //     unsafe {
    //         let mut this = Pin::new_unchecked(self);
    //         Pin::new_unchecked(Box::new(futures::future::poll_fn(move |cx| {
    //             //let this = &mut this;
    //             Poll::Ready(this.get_unchecked_mut().poll_ready(cx))
    //         })))
    //     }
    // }
}

/// Creates new `Service` values.
///
/// Acts as a service factory. This is useful for cases where new `Service`
/// values must be produced. One case is a TCP server listener. The listener
/// accepts new TCP streams, obtains a new `Service` value using the
/// `NewService` trait, and uses that new `Service` value to process inbound
/// requests on that new TCP stream.
///
/// `Config` is a service factory configuration type.
pub trait NewService {
    /// Requests handled by the service.
    type Request;

    /// Responses given by the service
    type Response;

    /// Errors produced by the service
    type Error;

    /// Service factory configuration
    type Config;

    /// The `Service` value created by this factory
    type Service: Service<
        Request = Self::Request,
        Response = Self::Response,
        Error = Self::Error,
    >;

    /// Errors produced while building a service.
    type InitError;

    /// The future of the `Service` instance.
    type Future: Future<Output = Result<Self::Service, Self::InitError>>;

    /// Create and return a new service value asynchronously.
    fn new_service(&self, cfg: &Self::Config) -> Self::Future;
}

impl<'a, S> Service for &'a mut S
where
    S: Service + 'a,
{
    type Request = S::Request;
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(&mut self, ctx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        (**self).poll_ready(ctx)
    }

    fn call(&mut self, request: Self::Request) -> S::Future {
        (**self).call(request)
    }
}

impl<S> Service for Box<S>
where
    S: Service + ?Sized,
{
    type Request = S::Request;
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(&mut self, ctx: &mut Context<'_>) -> Poll<Result<(), S::Error>> {
        (**self).poll_ready(ctx)
    }

    fn call(&mut self, request: Self::Request) -> S::Future {
        (**self).call(request)
    }
}

impl<S> Service for Rc<RefCell<S>>
where
    S: Service,
{
    type Request = S::Request;
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(&mut self, ctx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.borrow_mut().poll_ready(ctx)
    }

    fn call(&mut self, request: Self::Request) -> S::Future {
        (&mut (**self).borrow_mut()).call(request)
    }
}

impl<S> NewService for Rc<S>
where
    S: NewService,
{
    type Request = S::Request;
    type Response = S::Response;
    type Error = S::Error;
    type Config = S::Config;
    type Service = S::Service;
    type InitError = S::InitError;
    type Future = S::Future;

    fn new_service(&self, cfg: &S::Config) -> S::Future {
        self.as_ref().new_service(cfg)
    }
}

impl<S> NewService for Arc<S>
where
    S: NewService,
{
    type Request = S::Request;
    type Response = S::Response;
    type Error = S::Error;
    type Config = S::Config;
    type Service = S::Service;
    type InitError = S::InitError;
    type Future = S::Future;

    fn new_service(&self, cfg: &S::Config) -> S::Future {
        self.as_ref().new_service(cfg)
    }
}

/// Trait for types that can be converted to a `Service`
pub trait IntoService<T>
where
    T: Service,
{
    /// Convert to a `Service`
    fn into_service(self) -> T;
}

/// Trait for types that can be converted to a `NewService`
pub trait IntoNewService<T>
where
    T: NewService,
{
    /// Convert to an `NewService`
    fn into_new_service(self) -> T;
}

impl<T> IntoService<T> for T
where
    T: Service,
{
    fn into_service(self) -> T {
        self
    }
}

impl<T> IntoNewService<T> for T
where
    T: NewService,
{
    fn into_new_service(self) -> T {
        self
    }
}
