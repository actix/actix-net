use std::{
    future::Future,
    io,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use actix_codec::{AsyncRead, AsyncWrite};
use actix_service::{Service, ServiceFactory};
use actix_utils::counter::{Counter, CounterGuard};
use futures_core::future::LocalBoxFuture;
use tokio_rustls::{Accept, TlsAcceptor};

pub use rustls::{ServerConfig, Session};
pub use tokio_rustls::server::TlsStream;

use super::MAX_CONN_COUNTER;

/// Accept TLS connections via `rustls` package.
///
/// `rustls` feature enables this `Acceptor` type.
pub struct Acceptor {
    config: Arc<ServerConfig>,
}

impl Acceptor {
    /// Create Rustls based `Acceptor` service factory.
    #[inline]
    pub fn new(config: ServerConfig) -> Self {
        Acceptor {
            config: Arc::new(config),
        }
    }
}

impl Clone for Acceptor {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
        }
    }
}

impl<T> ServiceFactory<T> for Acceptor
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    type Response = TlsStream<T>;
    type Error = io::Error;
    type Config = ();

    type Service = AcceptorService;
    type InitError = ();
    type Future = LocalBoxFuture<'static, Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: ()) -> Self::Future {
        let res = MAX_CONN_COUNTER.with(|conns| {
            Ok(AcceptorService {
                acceptor: self.config.clone().into(),
                conns: conns.clone(),
            })
        });
        Box::pin(async { res })
    }
}

/// Rustls based `Acceptor` service
pub struct AcceptorService {
    acceptor: TlsAcceptor,
    conns: Counter,
}

impl<T> Service<T> for AcceptorService
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    type Response = TlsStream<T>;
    type Error = io::Error;
    type Future = AcceptorServiceFut<T>;

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        if self.conns.available(cx) {
            Poll::Ready(Ok(()))
        } else {
            Poll::Pending
        }
    }

    fn call(&self, req: T) -> Self::Future {
        AcceptorServiceFut {
            _guard: self.conns.get(),
            fut: self.acceptor.accept(req),
        }
    }
}

pub struct AcceptorServiceFut<T>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    fut: Accept<T>,
    _guard: CounterGuard,
}

impl<T> Future for AcceptorServiceFut<T>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    type Output = Result<TlsStream<T>, io::Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        Pin::new(&mut this.fut).poll(cx)
    }
}
