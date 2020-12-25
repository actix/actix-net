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
pub fn pipeline<U, S, Req>(service: U) -> Pipeline<S, Req>
where
    U: IntoService<S, Req>,
    S: Service<Req>,
{
    Pipeline {
        service: service.into_service(),
        _phantom: PhantomData,
    }
}

/// Construct new pipeline factory with one service factory.
pub fn pipeline_factory<U, SF, Req>(factory: U) -> PipelineFactory<SF, Req>
where
    U: IntoServiceFactory<SF, Req>,
    SF: ServiceFactory<Req>,
{
    PipelineFactory {
        factory: factory.into_factory(),
        _phantom: PhantomData,
    }
}

/// Pipeline service - pipeline allows to compose multiple service into one service.
pub struct Pipeline<S, Req> {
    service: S,
    _phantom: PhantomData<Req>,
}

impl<S, Req> Pipeline<S, Req>
where
    S: Service<Req>,
{
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
    ) -> Pipeline<impl Service<Req, Response = U::Response, Error = S::Error> + Clone, Req>
    where
        Self: Sized,
        F: IntoService<U, S::Response>,
        U: Service<S::Response, Error = S::Error>,
    {
        Pipeline {
            service: AndThenService::new(self.service, service.into_service()),
            _phantom: PhantomData,
        }
    }

    /// Apply function to specified service and use it as a next service in chain.
    ///
    /// Short version of `pipeline_factory(...).and_then(apply_fn(...))`
    pub fn and_then_apply_fn<U, S1, F, Fut, In, Res, Err>(
        self,
        service: U,
        wrap_fn: F,
    ) -> Pipeline<impl Service<Req, Response = Res, Error = Err> + Clone, Req>
    where
        Self: Sized,
        U: IntoService<S1, In>,
        S1: Service<In>,
        F: FnMut(S::Response, &mut S1) -> Fut,
        Fut: Future<Output = Result<Res, Err>>,
        Err: From<S::Error> + From<S1::Error>,
    {
        Pipeline {
            service: AndThenApplyFn::new(self.service, service.into_service(), wrap_fn),
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
    ) -> Pipeline<impl Service<Req, Response = U::Response, Error = S::Error> + Clone, Req>
    where
        Self: Sized,
        F: IntoService<U, Result<S::Response, S::Error>>,
        U: Service<Result<S::Response, S::Error>, Error = S::Error>,
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
    pub fn map<F, R>(self, f: F) -> Pipeline<Map<S, F, Req, R>, Req>
    where
        Self: Sized,
        F: FnMut(S::Response) -> R,
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
    pub fn map_err<F, E>(self, f: F) -> Pipeline<MapErr<S, Req, F, E>, Req>
    where
        Self: Sized,
        F: Fn(S::Error) -> E,
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
pub struct PipelineFactory<SF, Req> {
    factory: SF,
    _phantom: PhantomData<Req>,
}

impl<SF, Req> PipelineFactory<SF, Req>
where
    SF: ServiceFactory<Req>,
{
    /// Call another service after call to this one has resolved successfully.
    pub fn and_then<F, U>(
        self,
        factory: F,
    ) -> PipelineFactory<
        impl ServiceFactory<
                Req,
                Response = U::Response,
                Error = SF::Error,
                Config = SF::Config,
                InitError = SF::InitError,
                Service = impl Service<Req, Response = U::Response, Error = SF::Error> + Clone,
            > + Clone,
        Req,
    >
    where
        Self: Sized,
        SF::Config: Clone,
        F: IntoServiceFactory<U, SF::Response>,
        U: ServiceFactory<
            SF::Response,
            Config = SF::Config,
            Error = SF::Error,
            InitError = SF::InitError,
        >,
    {
        PipelineFactory {
            factory: AndThenServiceFactory::new(self.factory, factory.into_factory()),
            _phantom: PhantomData,
        }
    }

    /// Apply function to specified service and use it as a next service in chain.
    ///
    /// Short version of `pipeline_factory(...).and_then(apply_fn_factory(...))`
    pub fn and_then_apply_fn<U, SF1, Fut, F, In, Res, Err>(
        self,
        factory: U,
        wrap_fn: F,
    ) -> PipelineFactory<
        impl ServiceFactory<
                Req,
                Response = Res,
                Error = Err,
                Config = SF::Config,
                InitError = SF::InitError,
                Service = impl Service<Req, Response = Res, Error = Err> + Clone,
            > + Clone,
        Req,
    >
    where
        Self: Sized,
        SF::Config: Clone,
        U: IntoServiceFactory<SF1, In>,
        SF1: ServiceFactory<In, Config = SF::Config, InitError = SF::InitError>,
        F: FnMut(SF::Response, &mut SF1::Service) -> Fut + Clone,
        Fut: Future<Output = Result<Res, Err>>,
        Err: From<SF::Error> + From<SF1::Error>,
    {
        PipelineFactory {
            factory: AndThenApplyFnFactory::new(self.factory, factory.into_factory(), wrap_fn),
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
                Req,
                Response = U::Response,
                Error = SF::Error,
                Config = SF::Config,
                InitError = SF::InitError,
                Service = impl Service<Req, Response = U::Response, Error = SF::Error> + Clone,
            > + Clone,
        Req,
    >
    where
        Self: Sized,
        SF::Config: Clone,
        F: IntoServiceFactory<U, Result<SF::Response, SF::Error>>,
        U: ServiceFactory<
            Result<SF::Response, SF::Error>,
            Config = SF::Config,
            Error = SF::Error,
            InitError = SF::InitError,
        >,
    {
        PipelineFactory {
            factory: ThenServiceFactory::new(self.factory, factory.into_factory()),
            _phantom: PhantomData,
        }
    }

    /// Map this service's output to a different type, returning a new service
    /// of the resulting type.
    pub fn map<F, R>(self, f: F) -> PipelineFactory<MapServiceFactory<SF, F, Req, R>, Req>
    where
        Self: Sized,
        F: FnMut(SF::Response) -> R + Clone,
    {
        PipelineFactory {
            factory: MapServiceFactory::new(self.factory, f),
            _phantom: PhantomData,
        }
    }

    /// Map this service's error to a different error, returning a new service.
    pub fn map_err<F, E>(
        self,
        f: F,
    ) -> PipelineFactory<MapErrServiceFactory<SF, Req, F, E>, Req>
    where
        Self: Sized,
        F: Fn(SF::Error) -> E + Clone,
    {
        PipelineFactory {
            factory: MapErrServiceFactory::new(self.factory, f),
            _phantom: PhantomData,
        }
    }

    /// Map this factory's init error to a different error, returning a new service.
    pub fn map_init_err<F, E>(self, f: F) -> PipelineFactory<MapInitErr<SF, F, Req, E>, Req>
    where
        Self: Sized,
        F: Fn(SF::InitError) -> E + Clone,
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
