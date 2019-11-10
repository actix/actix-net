use std::task::{Context, Poll};

use crate::and_then::{AndThen, AndThenNewService};
use crate::then::{Then, ThenNewService};
use crate::{IntoNewService, IntoService, NewService, Service};

pub fn pipeline<F, T>(service: F) -> Pipeline<T>
where
    F: IntoService<T>,
    T: Service,
{
    Pipeline {
        service: service.into_service(),
    }
}

pub fn new_pipeline<F, T>(new_service: F) -> NewPipeline<T>
where
    F: IntoNewService<T>,
    T: NewService,
{
    NewPipeline {
        service: new_service.into_new_service(),
    }
}

/// Pipeline service
pub struct Pipeline<T> {
    service: T,
}

impl<T: Service> Pipeline<T> {
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
    ) -> Pipeline<impl Service<Request = T::Request, Response = U::Response, Error = T::Error>>
    where
        Self: Sized,
        F: IntoService<U>,
        U: Service<Request = T::Response, Error = T::Error>,
    {
        Pipeline {
            service: AndThen::new(self.service, service.into_service()),
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
    ) -> Pipeline<impl Service<Request = T::Request, Response = U::Response, Error = T::Error>>
    where
        Self: Sized,
        F: IntoService<U>,
        U: Service<Request = Result<T::Response, T::Error>, Error = T::Error>,
    {
        Pipeline {
            service: Then::new(self.service, service.into_service()),
        }
    }
}

impl<T: Service> Service for Pipeline<T> {
    type Request = T::Request;
    type Response = T::Response;
    type Error = T::Error;
    type Future = T::Future;

    #[inline]
    fn poll_ready(&mut self, ctx: &mut Context<'_>) -> Poll<Result<(), T::Error>> {
        self.service.poll_ready(ctx)
    }

    #[inline]
    fn call(&mut self, req: T::Request) -> Self::Future {
        self.service.call(req)
    }
}

/// Pipeline constructor
pub struct NewPipeline<T> {
    service: T,
}

impl<T: NewService> NewPipeline<T> {
    /// Call another service after call to this one has resolved successfully.
    pub fn and_then<F, U>(self, new_service: U) -> NewPipeline<AndThenNewService<T, U>>
    where
        Self: Sized,
        F: IntoNewService<U>,
        U: NewService<
            Config = T::Config,
            Request = T::Response,
            Error = T::Error,
            InitError = T::InitError,
        >,
    {
        NewPipeline {
            service: AndThenNewService::new(self.service, new_service.into_new_service()),
        }
    }

    /// Create `NewService` to chain on a computation for when a call to the
    /// service finished, passing the result of the call to the next
    /// service `U`.
    ///
    /// Note that this function consumes the receiving pipeline and returns a
    /// wrapped version of it.
    pub fn then<F, U>(self, new_service: F) -> NewPipeline<ThenNewService<T, U>>
    where
        Self: Sized,
        F: IntoNewService<U>,
        U: NewService<
            Config = T::Config,
            Request = Result<T::Response, T::Error>,
            Error = T::Error,
            InitError = T::InitError,
        >,
    {
        NewPipeline {
            service: ThenNewService::new(self.service, new_service.into_new_service()),
        }
    }
}

impl<T: NewService> NewService for NewPipeline<T> {
    type Config = T::Config;
    type Request = T::Request;
    type Response = T::Response;
    type Error = T::Error;
    type Service = T::Service;
    type InitError = T::InitError;
    type Future = T::Future;

    #[inline]
    fn new_service(&self, cfg: &T::Config) -> Self::Future {
        self.service.new_service(cfg)
    }
}
