use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures_util::future::FutureExt;

use crate::{Service, ServiceFactory};

pub type BoxFuture<I, E> = Pin<Box<dyn Future<Output = Result<I, E>>>>;

pub type BoxService<Req, Res, Err> =
    Box<dyn Service<Req, Response = Res, Error = Err, Future = BoxFuture<Res, Err>>>;

pub struct BoxServiceFactory<Cfg, Req, Res, Err, InitErr>(Inner<Cfg, Req, Res, Err, InitErr>);

/// Create boxed service factory
pub fn factory<T, Req>(
    factory: T,
) -> BoxServiceFactory<T::Config, Req, T::Response, T::Error, T::InitError>
where
    T: ServiceFactory<Req> + 'static,
    Req: 'static,
    T::Response: 'static,
    T::Service: 'static,
    T::Future: 'static,
    T::Error: 'static,
    T::InitError: 'static,
{
    BoxServiceFactory(Box::new(FactoryWrapper {
        factory,
        _t: std::marker::PhantomData,
    }))
}

/// Create boxed service
pub fn service<S, Req>(service: S) -> BoxService<Req, S::Response, S::Error>
where
    S: Service<Req> + 'static,
    S::Future: 'static,
{
    Box::new(ServiceWrapper(service))
}

type Inner<C, Req, Res, Err, InitErr> = Box<
    dyn ServiceFactory<
        Req,
        Config = C,
        Response = Res,
        Error = Err,
        InitError = InitErr,
        Service = BoxService<Req, Res, Err>,
        Future = BoxFuture<BoxService<Req, Res, Err>, InitErr>,
    >,
>;

impl<C, Req, Res, Err, InitErr> ServiceFactory<Req>
    for BoxServiceFactory<C, Req, Res, Err, InitErr>
where
    Req: 'static,
    Res: 'static,
    Err: 'static,
    InitErr: 'static,
{
    type Response = Res;
    type Error = Err;
    type InitError = InitErr;
    type Config = C;
    type Service = BoxService<Req, Res, Err>;

    type Future = BoxFuture<Self::Service, InitErr>;

    fn new_service(&self, cfg: C) -> Self::Future {
        self.0.new_service(cfg)
    }
}

struct FactoryWrapper<C, T: ServiceFactory<Req>, Req> {
    factory: T,
    _t: std::marker::PhantomData<C>,
}

impl<C, T, Req, Res, Err, InitErr> ServiceFactory<Req> for FactoryWrapper<C, T, Req>
where
    Req: 'static,
    Res: 'static,
    Err: 'static,
    InitErr: 'static,
    T: ServiceFactory<Req, Config = C, Response = Res, Error = Err, InitError = InitErr>,
    T::Future: 'static,
    T::Service: 'static,
    <T::Service as Service<Req>>::Future: 'static,
{
    type Response = Res;
    type Error = Err;
    type InitError = InitErr;
    type Config = C;
    type Service = BoxService<Req, Res, Err>;
    type Future = BoxFuture<Self::Service, Self::InitError>;

    fn new_service(&self, cfg: C) -> Self::Future {
        Box::pin(
            self.factory
                .new_service(cfg)
                .map(|res| res.map(ServiceWrapper::boxed)),
        )
    }
}

struct ServiceWrapper<S: Service<Req>, Req>(S);

impl<S, Req> ServiceWrapper<S, Req>
where
    S: Service<Req> + 'static,
    S::Future: 'static,
{
    fn boxed(service: S) -> BoxService<Req, S::Response, S::Error> {
        Box::new(ServiceWrapper(service))
    }
}

impl<T, Req, Res, Err> Service<Req> for ServiceWrapper<T, Req>
where
    T: Service<Req, Response = Res, Error = Err>,
    T::Future: 'static,
{
    type Response = Res;
    type Error = Err;
    type Future = BoxFuture<Res, Err>;

    fn poll_ready(&mut self, ctx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.0.poll_ready(ctx)
    }

    fn call(&mut self, req: Req) -> Self::Future {
        Box::pin(self.0.call(req))
    }
}
