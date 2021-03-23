//! Trait object forms of services and service factories.

use alloc::{boxed::Box, rc::Rc};
use core::{future::Future, pin::Pin};

use crate::{Service, ServiceFactory};

/// A boxed future without a Send bound or lifetime parameters.
pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T>>>;

macro_rules! service_object {
    ($name: ident, $type: tt, $fn_name: ident) => {
        /// Type alias for service trait object.
        pub type $name<Req, Res, Err> = $type<
            dyn Service<Req, Response = Res, Error = Err, Future = BoxFuture<Result<Res, Err>>>,
        >;

        /// Create service trait object.
        pub fn $fn_name<S, Req>(service: S) -> $name<Req, S::Response, S::Error>
        where
            S: Service<Req> + 'static,
            Req: 'static,
            S::Future: 'static,
        {
            $type::new(ServiceWrapper::new(service))
        }
    };
}

service_object!(BoxService, Box, service);
service_object!(RcService, Rc, rc_service);

struct ServiceWrapper<S> {
    inner: S,
}

impl<S> ServiceWrapper<S> {
    fn new(inner: S) -> Self {
        Self { inner }
    }
}

impl<S, Req, Res, Err> Service<Req> for ServiceWrapper<S>
where
    S: Service<Req, Response = Res, Error = Err>,
    S::Future: 'static,
{
    type Response = Res;
    type Error = Err;
    type Future = BoxFuture<Result<Res, Err>>;

    crate::forward_ready!(inner);

    fn call(&self, req: Req) -> Self::Future {
        Box::pin(self.inner.call(req))
    }
}

/// Wrapper for a service factory trait object that will produce a boxed trait object service.
pub struct BoxServiceFactory<Cfg, Req, Res, Err, InitErr>(Inner<Cfg, Req, Res, Err, InitErr>);

/// Create service factory trait object.
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
    BoxServiceFactory(Box::new(FactoryWrapper(factory)))
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
    type Config = C;
    type Service = BoxService<Req, Res, Err>;
    type InitError = InitErr;

    type Future = BoxFuture<Result<Self::Service, InitErr>>;

    fn new_service(&self, cfg: C) -> Self::Future {
        self.0.new_service(cfg)
    }
}

struct FactoryWrapper<SF>(SF);

impl<SF, Req, Cfg, Res, Err, InitErr> ServiceFactory<Req> for FactoryWrapper<SF>
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
    type Config = Cfg;
    type Service = BoxService<Req, Res, Err>;
    type InitError = InitErr;
    type Future = BoxFuture<Result<Self::Service, Self::InitError>>;

    fn new_service(&self, cfg: Cfg) -> Self::Future {
        let f = self.0.new_service(cfg);
        Box::pin(async { f.await.map(|s| Box::new(ServiceWrapper::new(s)) as _) })
    }
}
