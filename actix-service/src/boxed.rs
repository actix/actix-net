use std::pin::Pin;

use crate::{IntoFuture, NewService, Service, ServiceExt};
use futures::future::FutureExt;
use futures::future::LocalBoxFuture;
use futures::future::{err, ok, Either, Ready};
use futures::{Future, Poll};
use std::task::Context;

pub type BoxedService<Req, Res, Err> = Box<
    dyn Service<
        Request = Req,
        Response = Res,
        Error = Err,
        Future = BoxedServiceResponse<Res, Err>,
    >,
>;

pub type BoxedServiceResponse<Res, Err> =
    Either<Ready<Result<Res, Err>>, LocalBoxFuture<'static, Result<Res, Err>>>;

pub struct BoxedNewService<C, Req, Res, Err, InitErr>(Inner<C, Req, Res, Err, InitErr>);

/// Create boxed new service
pub fn new_service<T>(
    service: T,
) -> BoxedNewService<T::Config, T::Request, T::Response, T::Error, T::InitError>
where
    T: NewService + 'static,
    T::Request: 'static,
    T::Response: 'static,
    T::Service: 'static,
    T::Future: 'static,
    T::Error: 'static,
    T::InitError: 'static,
{
    BoxedNewService(Box::new(NewServiceWrapper {
        service,
        _t: std::marker::PhantomData,
    }))
}

/// Create boxed service
pub fn service<T>(service: T) -> BoxedService<T::Request, T::Response, T::Error>
where
    T: Service + 'static,
    T::Future: 'static,
{
    Box::new(ServiceWrapper(service))
}

type Inner<C, Req, Res, Err, InitErr> = Box<
    dyn NewService<
        Config = C,
        Request = Req,
        Response = Res,
        Error = Err,
        InitError = InitErr,
        Service = BoxedService<Req, Res, Err>,
        Future = LocalBoxFuture<'static, Result<BoxedService<Req, Res, Err>, InitErr>>,
    >,
>;

impl<C, Req, Res, Err, InitErr> NewService for BoxedNewService<C, Req, Res, Err, InitErr>
where
    Req: 'static,
    Res: 'static,
    Err: 'static,
    InitErr: 'static,
{
    type Request = Req;
    type Response = Res;
    type Error = Err;
    type InitError = InitErr;
    type Config = C;
    type Service = BoxedService<Req, Res, Err>;

    type Future = LocalBoxFuture<'static, Result<Self::Service, InitErr>>;

    fn new_service(&self, cfg: &C) -> Self::Future {
        self.0.new_service(cfg)
    }
}

struct NewServiceWrapper<C, T: NewService> {
    service: T,
    _t: std::marker::PhantomData<C>,
}

impl<C, T, Req, Res, Err, InitErr> NewService for NewServiceWrapper<C, T>
where
    Req: 'static,
    Res: 'static,
    Err: 'static,
    InitErr: 'static,
    T: NewService<Config = C, Request = Req, Response = Res, Error = Err, InitError = InitErr>,
    T::Future: 'static,
    T::Service: 'static,
    <T::Service as Service>::Future: 'static,
{
    type Request = Req;
    type Response = Res;
    type Error = Err;
    type InitError = InitErr;
    type Config = C;
    type Service = BoxedService<Req, Res, Err>;
    type Future = LocalBoxFuture<'static, Result<Self::Service, Self::InitError>>;

    fn new_service(&self, cfg: &C) -> Self::Future {
        /* TODO: Figure out what the hell is hapenning here
         Box::new(
            self.service
                .new_service(cfg)
                .into_future()
                .map(ServiceWrapper::boxed),
        )
        */
        unimplemented!()
    }
}

struct ServiceWrapper<T: Service>(T);

impl<T> ServiceWrapper<T>
where
    T: Service + 'static,
    T::Future: 'static,
{
    fn boxed(service: T) -> BoxedService<T::Request, T::Response, T::Error> {
        Box::new(ServiceWrapper(service))
    }
}

impl<T, Req, Res, Err> Service for ServiceWrapper<T>
where
    T: Service<Request = Req, Response = Res, Error = Err>,
    T::Future: 'static,
{
    type Request = Req;
    type Response = Res;
    type Error = Err;
    type Future = Either<
        Ready<Result<Self::Response, Self::Error>>,
        LocalBoxFuture<'static, Result<Res, Err>>,
    >;

    fn poll_ready(
        self: Pin<&mut Self>,
        ctx: &mut Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        unimplemented!()
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        unimplemented!()
    }

    /*
    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.0.poll_ready()
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        let mut fut = self.0.call(req);
        match fut.poll() {
            Ok(Async::Ready(res)) => Either::A(ok(res)),
            Err(e) => Either::A(err(e)),
            Ok(Async::NotReady) => Either::B(Box::new(fut)),
        }
    }
    */
}
