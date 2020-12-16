use std::task::{Context, Poll};
use std::{future::Future, marker::PhantomData};

use crate::and_then::{AndThenService, AndThenServiceFactory};
use crate::and_then_apply_fn::{AndThenApplyFn, AndThenApplyFnFactory};
use crate::map::{Map, MapServiceFactory};
use crate::map_err::{MapErr, MapErrServiceFactory};
use crate::map_init_err::MapInitErr;
use crate::then::{ThenService, ThenServiceFactory};
use crate::{IntoService, IntoServiceFactory, Service, ServiceFactory};

/// Construct new pipeline with one service in pipeline chain.
pub fn pipeline<F, T, Req>(service: F) -> Pipeline<T, Req>
where
    F: IntoService<T, Req>,
    T: Service<Req>,
{
    Pipeline {
        service: service.into_service(),
        _phantom: PhantomData,
    }
}

/// Construct new pipeline factory with one service factory.
pub fn pipeline_factory<T, F, Req>(factory: F) -> PipelineFactory<T, Req>
where
    T: ServiceFactory<Req>,
    F: IntoServiceFactory<T, Req>,
{
    PipelineFactory {
        factory: factory.into_factory(),
        _phantom: PhantomData,
    }
}

/// Pipeline service - pipeline allows to compose multiple service into one service.
pub struct Pipeline<T, Req> {
    service: T,
    _phantom: PhantomData<Req>,
}

impl<T: Service<Req>, Req> Pipeline<T, Req> {
    /// Call another service after call to this one has resolved successfully.
    ///
    /// This function can be used to chain two services together and ensure that
    /// the second service isn't called until call to the fist service have
    /// finished. Result of the call to the first service is used as an
    /// input parameter for the second service's call.
    ///
    /// Note that this function consumes the receiving service and returns a
    /// wrapped version of it.
    pub fn and_then<F, U>(
        self,
        service: F,
    ) -> Pipeline<impl Service<Req, Response = U::Response, Error = T::Error> + Clone, Req>
    where
        Self: Sized,
        F: IntoService<U, Req>,
        U: Service<T::Response, Error = T::Error>,
    {
        Pipeline {
            service: AndThenService::new(self.service, service.into_service()),
            _phantom: PhantomData,
        }
    }

    /// Apply function to specified service and use it as a next service in
    /// chain.
    ///
    /// Short version of `pipeline_factory(...).and_then(apply_fn_factory(...))`
    pub fn and_then_apply_fn<U, I, F, Fut, Res, Err>(
        self,
        service: I,
        f: F,
    ) -> Pipeline<impl Service<Req, Response = Res, Error = Err> + Clone, Req>
    where
        Self: Sized,
        I: IntoService<U, Req>,
        U: Service<Req>,
        F: FnMut(T::Response, &mut U) -> Fut,
        Fut: Future<Output = Result<Res, Err>>,
        Err: From<T::Error> + From<U::Error>,
    {
        Pipeline {
            service: AndThenApplyFn::new(self.service, service.into_service(), f),
            _phantom: PhantomData,
        }
    }

    /// Chain on a computation for when a call to the service finished,
    /// passing the result of the call to the next service `U`.
    ///
    /// Note that this function consumes the receiving pipeline and returns a
    /// wrapped version of it.
    pub fn then<F, U>(
        self,
        service: F,
    ) -> Pipeline<impl Service<Req, Response = U::Response, Error = T::Error> + Clone, Req>
    where
        Self: Sized,
        F: IntoService<U, Req>,
        U: Service<Result<T::Response, T::Error>, Error = T::Error>,
    {
        Pipeline {
            service: ThenService::new(self.service, service.into_service()),
            _phantom: PhantomData,
        }
    }

    /// Map this service's output to a different type, returning a new service
    /// of the resulting type.
    ///
    /// This function is similar to the `Option::map` or `Iterator::map` where
    /// it will change the type of the underlying service.
    ///
    /// Note that this function consumes the receiving service and returns a
    /// wrapped version of it, similar to the existing `map` methods in the
    /// standard library.
    pub fn map<F, R>(self, f: F) -> Pipeline<Map<T, F, Req, R>, Req>
    where
        Self: Sized,
        F: FnMut(T::Response) -> R,
    {
        Pipeline {
            service: Map::new(self.service, f),
            _phantom: PhantomData,
        }
    }

    /// Map this service's error to a different error, returning a new service.
    ///
    /// This function is similar to the `Result::map_err` where it will change
    /// the error type of the underlying service. This is useful for example to
    /// ensure that services have the same error type.
    ///
    /// Note that this function consumes the receiving service and returns a
    /// wrapped version of it.
    pub fn map_err<F, E>(self, f: F) -> Pipeline<MapErr<T, Req, F, E>, Req>
    where
        Self: Sized,
        F: Fn(T::Error) -> E,
    {
        Pipeline {
            service: MapErr::new(self.service, f),
            _phantom: PhantomData,
        }
    }
}

impl<T, Req> Clone for Pipeline<T, Req>
where
    T: Clone,
{
    fn clone(&self) -> Self {
        Pipeline {
            service: self.service.clone(),
            _phantom: PhantomData,
        }
    }
}

impl<T: Service<Req>, Req> Service<Req> for Pipeline<T, Req> {
    type Response = T::Response;
    type Error = T::Error;
    type Future = T::Future;

    #[inline]
    fn poll_ready(&mut self, ctx: &mut Context<'_>) -> Poll<Result<(), T::Error>> {
        self.service.poll_ready(ctx)
    }

    #[inline]
    fn call(&mut self, req: Req) -> Self::Future {
        self.service.call(req)
    }
}

/// Pipeline factory
pub struct PipelineFactory<T, Req> {
    factory: T,
    _phantom: PhantomData<Req>,
}

impl<T: ServiceFactory<Req>, Req> PipelineFactory<T, Req> {
    /// Call another service after call to this one has resolved successfully.
    pub fn and_then<F, U>(
        self,
        factory: F,
    ) -> PipelineFactory<
        impl ServiceFactory<
                Request = T::Request,
                Response = U::Response,
                Error = T::Error,
                Config = T::Config,
                InitError = T::InitError,
                Service = impl Service<
                    Request = T::Request,
                    Response = U::Response,
                    Error = T::Error,
                > + Clone,
            > + Clone,
        Req,
    >
    where
        Self: Sized,
        T::Config: Clone,
        F: IntoServiceFactory<U, Req>,
        U: ServiceFactory<
            T::Response,
            Config = T::Config,
            Error = T::Error,
            InitError = T::InitError,
        >,
    {
        PipelineFactory {
            factory: AndThenServiceFactory::new(self.factory, factory.into_factory()),
            _phantom: PhantomData,
        }
    }

    /// Apply function to specified service and use it as a next service in
    /// chain.
    ///
    /// Short version of `pipeline_factory(...).and_then(apply_fn_factory(...))`
    pub fn and_then_apply_fn<U, I, F, Fut, Res, Err>(
        self,
        factory: I,
        f: F,
    ) -> PipelineFactory<
        impl ServiceFactory<
                Request = T::Request,
                Response = Res,
                Error = Err,
                Config = T::Config,
                InitError = T::InitError,
                Service = impl Service<Request = T::Request, Response = Res, Error = Err> + Clone,
            > + Clone,
        Req,
    >
    where
        Self: Sized,
        T::Config: Clone,
        I: IntoServiceFactory<U, Req>,
        U: ServiceFactory<Req, Config = T::Config, InitError = T::InitError>,
        F: FnMut(T::Response, &mut U::Service) -> Fut + Clone,
        Fut: Future<Output = Result<Res, Err>>,
        Err: From<T::Error> + From<U::Error>,
    {
        PipelineFactory {
            factory: AndThenApplyFnFactory::new(self.factory, factory.into_factory(), f),
            _phantom: PhantomData,
        }
    }

    /// Create `NewService` to chain on a computation for when a call to the
    /// service finished, passing the result of the call to the next
    /// service `U`.
    ///
    /// Note that this function consumes the receiving pipeline and returns a
    /// wrapped version of it.
    pub fn then<F, U>(
        self,
        factory: F,
    ) -> PipelineFactory<
        impl ServiceFactory<
                Request = T::Request,
                Response = U::Response,
                Error = T::Error,
                Config = T::Config,
                InitError = T::InitError,
                Service = impl Service<
                    Request = T::Request,
                    Response = U::Response,
                    Error = T::Error,
                > + Clone,
            > + Clone,
        Req,
    >
    where
        Self: Sized,
        T::Config: Clone,
        F: IntoServiceFactory<U, Req>,
        U: ServiceFactory<
            Result<T::Response, T::Error>,
            Config = T::Config,
            Error = T::Error,
            InitError = T::InitError,
        >,
    {
        PipelineFactory {
            factory: ThenServiceFactory::new(self.factory, factory.into_factory()),
            _phantom: PhantomData,
        }
    }

    /// Map this service's output to a different type, returning a new service
    /// of the resulting type.
    pub fn map<F, R>(self, f: F) -> PipelineFactory<MapServiceFactory<T, F, Req, R>, Req>
    where
        Self: Sized,
        F: FnMut(T::Response) -> R + Clone,
    {
        PipelineFactory {
            factory: MapServiceFactory::new(self.factory, f),
            _phantom: PhantomData,
        }
    }

    /// Map this service's error to a different error, returning a new service.
    pub fn map_err<F, E>(self, f: F) -> PipelineFactory<MapErrServiceFactory<T, Req, F, E>, Req>
    where
        Self: Sized,
        F: Fn(T::Error) -> E + Clone,
    {
        PipelineFactory {
            factory: MapErrServiceFactory::new(self.factory, f),
            _phantom: PhantomData,
        }
    }

    /// Map this factory's init error to a different error, returning a new service.
    pub fn map_init_err<F, E>(self, f: F) -> PipelineFactory<MapInitErr<T, F, Req, E>, Req>
    where
        Self: Sized,
        F: Fn(T::InitError) -> E + Clone,
    {
        PipelineFactory {
            factory: MapInitErr::new(self.factory, f),
            _phantom: PhantomData,
        }
    }
}

impl<T, Req> Clone for PipelineFactory<T, Req>
where
    T: Clone,
{
    fn clone(&self) -> Self {
        PipelineFactory {
            factory: self.factory.clone(),
            _phantom: PhantomData,
        }
    }
}

impl<T: ServiceFactory<Req>, Req> ServiceFactory<Req> for PipelineFactory<T, Req> {
    type Config = T::Config;
    type Response = T::Response;
    type Error = T::Error;
    type Service = T::Service;
    type InitError = T::InitError;
    type Future = T::Future;

    #[inline]
    fn new_service(&self, cfg: T::Config) -> Self::Future {
        self.factory.new_service(cfg)
    }
}
