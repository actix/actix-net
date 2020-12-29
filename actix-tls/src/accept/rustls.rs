use std::future::Future;
use std::io;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use actix_codec::{AsyncRead, AsyncWrite};
use actix_service::{Service, ServiceFactory};
use actix_utils::counter::{Counter, CounterGuard};
use futures_util::future::{ok, Ready};
use tokio_rustls::{Accept, TlsAcceptor};

pub use rustls::{ServerConfig, Session};
pub use tokio_rustls::server::TlsStream;
pub use webpki_roots::TLS_SERVER_ROOTS;

use super::MAX_CONN_COUNTER;

/// Accept TLS connections via `rustls` package.
///
/// `rustls` feature enables this `Acceptor` type.
pub struct Acceptor<T> {
    config: Arc<ServerConfig>,
    io: PhantomData<T>,
}

impl<T> Acceptor<T>
where
    T: AsyncRead + AsyncWrite,
{
    /// Create Rustls based `Acceptor` service factory.
    #[inline]
    pub fn new(config: ServerConfig) -> Self {
        Acceptor {
            config: Arc::new(config),
            io: PhantomData,
        }
    }
}

impl<T> Clone for Acceptor<T> {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            io: PhantomData,
        }
    }
}

impl<T> ServiceFactory<T> for Acceptor<T>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    type Response = TlsStream<T>;
    type Error = io::Error;
    type Service = AcceptorService<T>;

    type Config = ();
    type InitError = ();
    type Future = Ready<Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: ()) -> Self::Future {
        MAX_CONN_COUNTER.with(|conns| {
            ok(AcceptorService {
                acceptor: self.config.clone().into(),
                conns: conns.clone(),
                io: PhantomData,
            })
        })
    }
}

/// Rustls based `Acceptor` service
pub struct AcceptorService<T> {
    acceptor: TlsAcceptor,
    io: PhantomData<T>,
    conns: Counter,
}

impl<T> Service<T> for AcceptorService<T>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    type Response = TlsStream<T>;
    type Error = io::Error;
    type Future = AcceptorServiceFut<T>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        if self.conns.available(cx) {
            Poll::Ready(Ok(()))
        } else {
            Poll::Pending
        }
    }

    fn call(&mut self, req: T) -> Self::Future {
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

        let res = futures_util::ready!(Pin::new(&mut this.fut).poll(cx));
        match res {
            Ok(io) => Poll::Ready(Ok(io)),
            Err(e) => Poll::Ready(Err(e)),
        }
    }
}
