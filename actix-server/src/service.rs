use std::marker::PhantomData;
use std::net::SocketAddr;
use std::task::{Context, Poll};

use actix_service::{Service, ServiceFactory as BaseServiceFactory};
use actix_utils::future::{ready, Ready};
use futures_core::future::LocalBoxFuture;
use log::error;

use crate::socket::{FromConnection, MioStream};
use crate::worker::WorkerCounterGuard;

pub trait ServiceFactory<Io, C = MioStream>
where
    Io: FromConnection<C>,
    Self: Send + Clone + 'static,
{
    type Factory: BaseServiceFactory<Io, Config = ()>;

    fn create(&self) -> Self::Factory;
}

pub(crate) trait InternalServiceFactory<C>: Send {
    fn name(&self, token: usize) -> &str;

    fn clone_factory(&self) -> Box<dyn InternalServiceFactory<C>>;

    fn create(&self) -> LocalBoxFuture<'static, Result<(usize, BoxedServerService<C>), ()>>;
}

pub(crate) type BoxedServerService<C> = Box<
    dyn Service<
        (WorkerCounterGuard<C>, C),
        Response = (),
        Error = (),
        Future = Ready<Result<(), ()>>,
    >,
>;

pub(crate) struct StreamService<S, Io> {
    service: S,
    _phantom: PhantomData<Io>,
}

impl<S, Io> StreamService<S, Io> {
    pub(crate) fn new(service: S) -> Self {
        StreamService {
            service,
            _phantom: PhantomData,
        }
    }
}

impl<S, Io, C> Service<(WorkerCounterGuard<C>, C)> for StreamService<S, Io>
where
    S: Service<Io>,
    S::Future: 'static,
    S::Error: 'static,
    Io: FromConnection<C>,
    C: 'static,
{
    type Response = ();
    type Error = ();
    type Future = Ready<Result<(), ()>>;

    #[inline]
    fn poll_ready(&self, ctx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(ctx).map_err(|_| ())
    }

    fn call(&self, (guard, req): (WorkerCounterGuard<C>, C)) -> Self::Future {
        ready(match FromConnection::from_conn(req) {
            Ok(stream) => {
                let f = self.service.call(stream);
                actix_rt::spawn(async {
                    let _ = f.await;
                    drop(guard);
                });
                Ok(())
            }
            Err(e) => {
                error!("Can not convert to an async tcp stream: {}", e);
                Err(())
            }
        })
    }
}

pub(crate) struct StreamNewService<F, Io, C>
where
    F: ServiceFactory<Io, C>,
    Io: FromConnection<C> + Send,
{
    name: String,
    inner: F,
    token: usize,
    addr: SocketAddr,
    _t: PhantomData<(Io, C)>,
}

impl<F, Io, C> StreamNewService<F, Io, C>
where
    F: ServiceFactory<Io, C>,
    Io: FromConnection<C> + Send + 'static,
    C: Send + 'static,
{
    pub(crate) fn create(
        name: String,
        token: usize,
        inner: F,
        addr: SocketAddr,
    ) -> Box<dyn InternalServiceFactory<C>> {
        Box::new(Self {
            name,
            token,
            inner,
            addr,
            _t: PhantomData,
        })
    }
}

impl<F, Io, C> InternalServiceFactory<C> for StreamNewService<F, Io, C>
where
    F: ServiceFactory<Io, C>,
    Io: FromConnection<C> + Send + 'static,
    C: Send + 'static,
{
    fn name(&self, _: usize) -> &str {
        &self.name
    }

    fn clone_factory(&self) -> Box<dyn InternalServiceFactory<C>> {
        Box::new(Self {
            name: self.name.clone(),
            inner: self.inner.clone(),
            token: self.token,
            addr: self.addr,
            _t: PhantomData,
        })
    }

    fn create(&self) -> LocalBoxFuture<'static, Result<(usize, BoxedServerService<C>), ()>> {
        let token = self.token;
        let fut = self.inner.create().new_service(());
        Box::pin(async move {
            match fut.await {
                Ok(inner) => {
                    let service = Box::new(StreamService::new(inner)) as _;
                    Ok((token, service))
                }
                Err(_) => Err(()),
            }
        })
    }
}

impl<F, T, Io, C> ServiceFactory<Io, C> for F
where
    F: Fn() -> T + Send + Clone + 'static,
    T: BaseServiceFactory<Io, Config = ()>,
    Io: FromConnection<C>,
{
    type Factory = T;

    fn create(&self) -> T {
        (self)()
    }
}
