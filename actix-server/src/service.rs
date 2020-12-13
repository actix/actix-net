use std::marker::PhantomData;
use std::net::SocketAddr;
use std::task::{Context, Poll};

use actix_rt::spawn;
use actix_service::{self as actix, Service, ServiceFactory as ActixServiceFactory};
use actix_utils::counter::CounterGuard;
use futures_util::future::{err, ok, LocalBoxFuture, Ready};
use futures_util::{FutureExt, TryFutureExt};
use log::error;

use super::Token;
use crate::socket::{FromStream, StdStream};

pub trait ServiceFactory<Stream: FromStream>: Send + Clone + 'static {
    type Factory: actix::ServiceFactory<Config = (), Request = Stream>;

    fn create(&self) -> Self::Factory;
}

pub(crate) trait InternalServiceFactory: Send {
    fn name(&self, token: Token) -> &str;

    fn clone_factory(&self) -> Box<dyn InternalServiceFactory>;

    fn create(&self) -> LocalBoxFuture<'static, Result<Vec<(Token, BoxedServerService)>, ()>>;
}

pub(crate) type BoxedServerService = Box<
    dyn Service<
        Request = (Option<CounterGuard>, StdStream),
        Response = (),
        Error = (),
        Future = Ready<Result<(), ()>>,
    >,
>;

pub(crate) struct StreamService<T> {
    service: T,
}

impl<T> StreamService<T> {
    pub(crate) fn new(service: T) -> Self {
        StreamService { service }
    }
}

impl<T, I> Service for StreamService<T>
where
    T: Service<Request = I>,
    T::Future: 'static,
    T::Error: 'static,
    I: FromStream,
{
    type Request = (Option<CounterGuard>, StdStream);
    type Response = ();
    type Error = ();
    type Future = Ready<Result<(), ()>>;

    fn poll_ready(&mut self, ctx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(ctx).map_err(|_| ())
    }

    fn call(&mut self, (guard, req): (Option<CounterGuard>, StdStream)) -> Self::Future {
        match FromStream::from_stdstream(req) {
            Ok(stream) => {
                let f = self.service.call(stream);
                spawn(async move {
                    let _ = f.await;
                    drop(guard);
                });
                ok(())
            }
            Err(e) => {
                error!("Can not convert to an async tcp stream: {}", e);
                err(())
            }
        }
    }
}

pub(crate) struct StreamNewService<F: ServiceFactory<Io>, Io: FromStream> {
    name: String,
    inner: F,
    token: Token,
    addr: SocketAddr,
    _t: PhantomData<Io>,
}

impl<F, Io> StreamNewService<F, Io>
where
    F: ServiceFactory<Io>,
    Io: FromStream + Send + 'static,
{
    pub(crate) fn create(
        name: String,
        token: Token,
        inner: F,
        addr: SocketAddr,
    ) -> Box<dyn InternalServiceFactory> {
        Box::new(Self {
            name,
            token,
            inner,
            addr,
            _t: PhantomData,
        })
    }
}

impl<F, Io> InternalServiceFactory for StreamNewService<F, Io>
where
    F: ServiceFactory<Io>,
    Io: FromStream + Send + 'static,
{
    fn name(&self, _: Token) -> &str {
        &self.name
    }

    fn clone_factory(&self) -> Box<dyn InternalServiceFactory> {
        Box::new(Self {
            name: self.name.clone(),
            inner: self.inner.clone(),
            token: self.token,
            addr: self.addr,
            _t: PhantomData,
        })
    }

    fn create(&self) -> LocalBoxFuture<'static, Result<Vec<(Token, BoxedServerService)>, ()>> {
        let token = self.token;
        self.inner
            .create()
            .new_service(())
            .map_err(|_| ())
            .map_ok(move |inner| {
                let service: BoxedServerService = Box::new(StreamService::new(inner));
                vec![(token, service)]
            })
            .boxed_local()
    }
}

impl<F, T, I> ServiceFactory<I> for F
where
    F: Fn() -> T + Send + Clone + 'static,
    T: actix::ServiceFactory<Config = (), Request = I>,
    I: FromStream,
{
    type Factory = T;

    fn create(&self) -> T {
        (self)()
    }
}
