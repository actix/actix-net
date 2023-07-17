use std::{
    marker::PhantomData,
    net::SocketAddr,
    task::{Context, Poll},
};

use actix_service::{Service, ServiceFactory as BaseServiceFactory};
use actix_utils::future::{ready, Ready};
use futures_core::future::LocalBoxFuture;
use tracing::error;

use crate::{
    socket::{FromStream, MioStream},
    worker::WorkerCounterGuard,
};

#[doc(hidden)]
pub trait ServerServiceFactory<Stream: FromStream>: Send + Clone + 'static {
    type Factory: BaseServiceFactory<Stream, Config = ()>;

    fn create(&self) -> Self::Factory;
}

pub(crate) trait InternalServiceFactory: Send {
    fn name(&self, token: usize) -> &str;

    fn clone_factory(&self) -> Box<dyn InternalServiceFactory>;

    fn create(&self) -> LocalBoxFuture<'static, Result<(usize, BoxedServerService), ()>>;
}

pub(crate) type BoxedServerService = Box<
    dyn Service<
        (WorkerCounterGuard, MioStream),
        Response = (),
        Error = (),
        Future = Ready<Result<(), ()>>,
    >,
>;

pub(crate) struct StreamService<S, I> {
    service: S,
    _phantom: PhantomData<I>,
}

impl<S, I> StreamService<S, I> {
    pub(crate) fn new(service: S) -> Self {
        StreamService {
            service,
            _phantom: PhantomData,
        }
    }
}

impl<S, I> Service<(WorkerCounterGuard, MioStream)> for StreamService<S, I>
where
    S: Service<I>,
    S::Future: 'static,
    S::Error: 'static,
    I: FromStream,
{
    type Response = ();
    type Error = ();
    type Future = Ready<Result<(), ()>>;

    fn poll_ready(&self, ctx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(ctx).map_err(|_| ())
    }

    fn call(&self, (guard, req): (WorkerCounterGuard, MioStream)) -> Self::Future {
        ready(match FromStream::from_mio(req) {
            Ok(stream) => {
                let f = self.service.call(stream);
                actix_rt::spawn(async move {
                    let _ = f.await;
                    drop(guard);
                });
                Ok(())
            }
            Err(err) => {
                error!("can not convert to an async TCP stream: {err}");
                Err(())
            }
        })
    }
}

pub(crate) struct StreamNewService<F: ServerServiceFactory<Io>, Io: FromStream> {
    name: String,
    inner: F,
    token: usize,
    addr: SocketAddr,
    _t: PhantomData<Io>,
}

impl<F, Io> StreamNewService<F, Io>
where
    F: ServerServiceFactory<Io>,
    Io: FromStream + Send + 'static,
{
    pub(crate) fn create(
        name: String,
        token: usize,
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
    F: ServerServiceFactory<Io>,
    Io: FromStream + Send + 'static,
{
    fn name(&self, _: usize) -> &str {
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

    fn create(&self) -> LocalBoxFuture<'static, Result<(usize, BoxedServerService), ()>> {
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

impl<F, T, I> ServerServiceFactory<I> for F
where
    F: Fn() -> T + Send + Clone + 'static,
    T: BaseServiceFactory<I, Config = ()>,
    I: FromStream,
{
    type Factory = T;

    fn create(&self) -> T {
        (self)()
    }
}
