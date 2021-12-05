// TODO: see if pipeline is necessary
#![allow(dead_code)]

use core::{
    marker::PhantomData,
    task::{Context, Poll},
};

use crate::and_then::{AndThenService, AndThenServiceFactory};
use crate::map::{Map, MapServiceFactory};
use crate::map_err::{MapErr, MapErrServiceFactory};
use crate::map_init_err::MapInitErr;
use crate::then::{ThenService, ThenServiceFactory};
use crate::{IntoService, IntoServiceFactory, Service, ServiceFactory};

/// Construct new pipeline with one service in pipeline chain.
pub(crate) fn pipeline<I, S, Req>(service: I) -> Pipeline<S, Req>
where
    I: IntoService<S, Req>,
    S: Service<Req>,
{
    Pipeline {
        service: service.into_service(),
        _phantom: PhantomData,
    }
}

/// Construct new pipeline factory with one service factory.
pub(crate) fn pipeline_factory<I, SF, Req>(factory: I) -> PipelineFactory<SF, Req>
where
    I: IntoServiceFactory<SF, Req>,
    SF: ServiceFactory<Req>,
{
    PipelineFactory {
        factory: factory.into_factory(),
        _phantom: PhantomData,
    }
}

/// Pipeline service - pipeline allows to compose multiple service into one service.
pub(crate) struct Pipeline<S, Req> {
    service: S,
    _phantom: PhantomData<fn(Req)>,
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
    pub fn and_then<I, S1>(
        self,
        service: I,
    ) -> Pipeline<impl Service<Req, Response = S1::Response, Error = S::Error> + Clone, Req>
    where
        Self: Sized,
        I: IntoService<S1, S::Response>,
        S1: Service<S::Response, Error = S::Error>,
    {
        Pipeline {
            service: AndThenService::new(self.service, service.into_service()),
            _phantom: PhantomData,
        }
    }

    /// Chain on a computation for when a call to the service finished,
    /// passing the result of the call to the next service `U`.
    ///
    /// Note that this function consumes the receiving pipeline and returns a
    /// wrapped version of it.
    pub fn then<F, S1>(
        self,
        service: F,
    ) -> Pipeline<impl Service<Req, Response = S1::Response, Error = S::Error> + Clone, Req>
    where
        Self: Sized,
        F: IntoService<S1, Result<S::Response, S::Error>>,
        S1: Service<Result<S::Response, S::Error>, Error = S::Error>,
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

impl<S: Service<Req>, Req> Service<Req> for Pipeline<S, Req> {
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    #[inline]
    fn poll_ready(&self, ctx: &mut Context<'_>) -> Poll<Result<(), S::Error>> {
        self.service.poll_ready(ctx)
    }

    #[inline]
    fn call(&self, req: Req) -> Self::Future {
        self.service.call(req)
    }
}

/// Pipeline factory
pub(crate) struct PipelineFactory<SF, Req> {
    factory: SF,
    _phantom: PhantomData<fn(Req)>,
}

impl<SF, Req> PipelineFactory<SF, Req>
where
    SF: ServiceFactory<Req>,
{
    /// Call another service after call to this one has resolved successfully.
    pub fn and_then<I, SF1>(
        self,
        factory: I,
    ) -> PipelineFactory<
        impl ServiceFactory<
                Req,
                Response = SF1::Response,
                Error = SF::Error,
                Config = SF::Config,
                InitError = SF::InitError,
                Service = impl Service<Req, Response = SF1::Response, Error = SF::Error> + Clone,
            > + Clone,
        Req,
    >
    where
        Self: Sized,
        SF::Config: Clone,
        I: IntoServiceFactory<SF1, SF::Response>,
        SF1: ServiceFactory<
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

    /// Create `NewService` to chain on a computation for when a call to the
    /// service finished, passing the result of the call to the next
    /// service `U`.
    ///
    /// Note that this function consumes the receiving pipeline and returns a
    /// wrapped version of it.
    pub fn then<I, SF1>(
        self,
        factory: I,
    ) -> PipelineFactory<
        impl ServiceFactory<
                Req,
                Response = SF1::Response,
                Error = SF::Error,
                Config = SF::Config,
                InitError = SF::InitError,
                Service = impl Service<Req, Response = SF1::Response, Error = SF::Error> + Clone,
            > + Clone,
        Req,
    >
    where
        Self: Sized,
        SF::Config: Clone,
        I: IntoServiceFactory<SF1, Result<SF::Response, SF::Error>>,
        SF1: ServiceFactory<
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

impl<SF, Req> ServiceFactory<Req> for PipelineFactory<SF, Req>
where
    SF: ServiceFactory<Req>,
{
    type Config = SF::Config;
    type Response = SF::Response;
    type Error = SF::Error;
    type Service = SF::Service;
    type InitError = SF::InitError;
    type Future = SF::Future;

    #[inline]
    fn new_service(&self, cfg: SF::Config) -> Self::Future {
        self.factory.new_service(cfg)
    }
}
