use alloc::boxed::Box;
use core::{
    future::Future,
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

use crate::{Service, ServiceFactory};

pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T>>>;

pub type BoxService<Req, Res, Err> =
    Box<dyn Service<Req, Response = Res, Error = Err, Future = BoxFuture<Result<Res, Err>>>>;

pub struct BoxServiceFactory<Cfg, Req, Res, Err, InitErr>(Inner<Cfg, Req, Res, Err, InitErr>);

/// Create boxed service factory
pub fn factory<SF, Req>(
    factory: SF,
) -> BoxServiceFactory<SF::Config, Req, SF::Response, SF::Error, SF::InitError>
where
    SF: ServiceFactory<Req> + 'static,
    Req: 'static,
    SF::Response: 'static,
    SF::Service: 'static,
    SF::Future: 'static,
    SF::Error: 'static,
    SF::InitError: 'static,
{
    BoxServiceFactory(Box::new(FactoryWrapper {
        factory,
        _t: PhantomData,
    }))
}

/// Create boxed service
pub fn service<S, Req>(service: S) -> BoxService<Req, S::Response, S::Error>
where
    S: Service<Req> + 'static,
    Req: 'static,
    S::Future: 'static,
{
    Box::new(ServiceWrapper(service, PhantomData))
}

type Inner<C, Req, Res, Err, InitErr> = Box<
    dyn ServiceFactory<
        Req,
        Config = C,
        Response = Res,
        Error = Err,
        InitError = InitErr,
        Service = BoxService<Req, Res, Err>,
        Future = BoxFuture<Result<BoxService<Req, Res, Err>, InitErr>>,
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

    type Future = BoxFuture<Result<Self::Service, InitErr>>;

    fn new_service(&self, cfg: C) -> Self::Future {
        self.0.new_service(cfg)
    }
}

struct FactoryWrapper<SF, Req, Cfg> {
    factory: SF,
    _t: PhantomData<(Req, Cfg)>,
}

impl<SF, Req, Cfg, Res, Err, InitErr> ServiceFactory<Req> for FactoryWrapper<SF, Req, Cfg>
where
    Req: 'static,
    Res: 'static,
    Err: 'static,
    InitErr: 'static,
    SF: ServiceFactory<Req, Config = Cfg, Response = Res, Error = Err, InitError = InitErr>,
    SF::Future: 'static,
    SF::Service: 'static,
    <SF::Service as Service<Req>>::Future: 'static,
{
    type Response = Res;
    type Error = Err;
    type InitError = InitErr;
    type Config = Cfg;
    type Service = BoxService<Req, Res, Err>;
    type Future = BoxFuture<Result<Self::Service, Self::InitError>>;

    fn new_service(&self, cfg: Cfg) -> Self::Future {
        let fut = self.factory.new_service(cfg);
        Box::pin(async {
            let res = fut.await;
            res.map(ServiceWrapper::boxed)
        })
    }
}

struct ServiceWrapper<S: Service<Req>, Req>(S, PhantomData<Req>);

impl<S, Req> ServiceWrapper<S, Req>
where
    S: Service<Req> + 'static,
    Req: 'static,
    S::Future: 'static,
{
    fn boxed(service: S) -> BoxService<Req, S::Response, S::Error> {
        Box::new(ServiceWrapper(service, PhantomData))
    }
}

impl<S, Req, Res, Err> Service<Req> for ServiceWrapper<S, Req>
where
    S: Service<Req, Response = Res, Error = Err>,
    S::Future: 'static,
{
    type Response = Res;
    type Error = Err;
    type Future = BoxFuture<Result<Res, Err>>;

    fn poll_ready(&self, ctx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.0.poll_ready(ctx)
    }

    fn call(&self, req: Req) -> Self::Future {
        Box::pin(self.0.call(req))
    }
}
