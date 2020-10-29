use std::marker::PhantomData;
use std::net::SocketAddr;
use std::task::{Context, Poll};
use std::time::Duration;

use actix_rt::ExecFactory;
use actix_service::{self as actix, Service, ServiceFactory as ActixServiceFactory};
use actix_utils::counter::CounterGuard;
use futures_util::future::{err, ok, LocalBoxFuture, Ready};
use futures_util::{FutureExt, TryFutureExt};
use log::error;

use super::Token;
use crate::socket::{FromStream, StdStream};

/// Server message
pub(crate) enum ServerMessage {
    /// New stream
    Connect(StdStream),

    /// Gracefully shutdown
    Shutdown(Duration),

    /// Force shutdown
    ForceShutdown,
}

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
        Request = (Option<CounterGuard>, ServerMessage),
        Response = (),
        Error = (),
        Future = Ready<Result<(), ()>>,
    >,
>;

pub(crate) struct StreamService<T, Exec> {
    service: T,
    _exec: PhantomData<Exec>,
}

impl<T, Exec> StreamService<T, Exec> {
    pub(crate) fn new(service: T) -> Self {
        StreamService {
            service,
            _exec: PhantomData,
        }
    }
}

impl<T, I, Exec> Service for StreamService<T, Exec>
where
    T: Service<Request = I>,
    T::Future: 'static,
    T::Error: 'static,
    I: FromStream,
    Exec: ExecFactory,
{
    type Request = (Option<CounterGuard>, ServerMessage);
    type Response = ();
    type Error = ();
    type Future = Ready<Result<(), ()>>;

    fn poll_ready(&mut self, ctx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(ctx).map_err(|_| ())
    }

    fn call(&mut self, (guard, req): (Option<CounterGuard>, ServerMessage)) -> Self::Future {
        match req {
            ServerMessage::Connect(stream) => {
                let stream = FromStream::from_stdstream(stream).map_err(|e| {
                    error!("Can not convert to an async tcp stream: {}", e);
                });

                if let Ok(stream) = stream {
                    let f = self.service.call(stream);
                    Exec::spawn(async move {
                        let _ = f.await;
                        drop(guard);
                    });
                    ok(())
                } else {
                    err(())
                }
            }
            _ => ok(()),
        }
    }
}

pub(crate) struct StreamNewService<F, Io, Exec>
where
    F: ServiceFactory<Io>,
    Io: FromStream,
    Exec: ExecFactory,
{
    name: String,
    inner: F,
    token: Token,
    addr: SocketAddr,
    _t: PhantomData<(Io, Exec)>,
}

impl<F, Io, Exec> StreamNewService<F, Io, Exec>
where
    F: ServiceFactory<Io>,
    Io: FromStream + Send + 'static,
    Exec: ExecFactory,
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

impl<F, Io, Exec> InternalServiceFactory for StreamNewService<F, Io, Exec>
where
    F: ServiceFactory<Io>,
    Io: FromStream + Send + 'static,
    Exec: ExecFactory,
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
                let service: BoxedServerService =
                    Box::new(StreamService::<_, Exec>::new(inner));
                vec![(token, service)]
            })
            .boxed_local()
    }
}

impl InternalServiceFactory for Box<dyn InternalServiceFactory> {
    fn name(&self, token: Token) -> &str {
        self.as_ref().name(token)
    }

    fn clone_factory(&self) -> Box<dyn InternalServiceFactory> {
        self.as_ref().clone_factory()
    }

    fn create(&self) -> LocalBoxFuture<'static, Result<Vec<(Token, BoxedServerService)>, ()>> {
        self.as_ref().create()
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
