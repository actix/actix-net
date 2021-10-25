use std::{
    fmt,
    marker::PhantomData,
    net::SocketAddr,
    task::{Context, Poll},
};

use actix_service::{Service, ServiceFactory};
use actix_utils::future::{ready, Ready};
use futures_core::future::LocalBoxFuture;
use log::error;

use crate::{
    socket::{FromStream, MioStream},
    worker::WorkerCounterGuard,
};

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
            Err(e) => {
                error!("Can not convert to an async tcp stream: {}", e);
                Err(())
            }
        })
    }
}

pub(crate) struct StreamNewService<F, Io, InitErr> {
    name: String,
    inner: F,
    token: usize,
    addr: SocketAddr,
    _t: PhantomData<(Io, InitErr)>,
}

impl<F, Io, InitErr> StreamNewService<F, Io, InitErr>
where
    F: ServiceFactory<Io, Config = (), InitError = InitErr> + Send + Clone + 'static,
    InitErr: fmt::Debug + Send + 'static,
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

impl<F, Io, InitErr> InternalServiceFactory for StreamNewService<F, Io, InitErr>
where
    F: ServiceFactory<Io, Config = (), InitError = InitErr> + Send + Clone + 'static,
    InitErr: fmt::Debug + Send + 'static,
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
        let fut = self.inner.new_service(());
        Box::pin(async move {
            match fut.await {
                Ok(inner) => {
                    let service = Box::new(StreamService::new(inner)) as _;
                    Ok((token, service))
                }
                Err(err) => {
                    error!("{:?}", err);
                    Err(())
                }
            }
        })
    }
}
