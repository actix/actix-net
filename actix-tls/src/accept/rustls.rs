use std::{
    future::Future,
    io::{self, IoSlice},
    ops::{Deref, DerefMut},
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use actix_codec::{AsyncRead, AsyncWrite, ReadBuf};
use actix_rt::net::{ActixStream, Ready};
use actix_service::{Service, ServiceFactory};
use actix_utils::counter::{Counter, CounterGuard};
use futures_core::future::LocalBoxFuture;
use tokio_rustls::{Accept, TlsAcceptor};

pub use tokio_rustls::rustls::{ServerConfig, Session};

use super::MAX_CONN_COUNTER;

/// Wrapper type for `tokio_openssl::SslStream` in order to impl `ActixStream` trait.
pub struct TlsStream<T>(tokio_rustls::server::TlsStream<T>);

impl<T> From<tokio_rustls::server::TlsStream<T>> for TlsStream<T> {
    fn from(stream: tokio_rustls::server::TlsStream<T>) -> Self {
        Self(stream)
    }
}

impl<T> Deref for TlsStream<T> {
    type Target = tokio_rustls::server::TlsStream<T>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for TlsStream<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<T: ActixStream> AsyncRead for TlsStream<T> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut **self.get_mut()).poll_read(cx, buf)
    }
}

impl<T: ActixStream> AsyncWrite for TlsStream<T> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut **self.get_mut()).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut **self.get_mut()).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut **self.get_mut()).poll_shutdown(cx)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut **self.get_mut()).poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        (&**self).is_write_vectored()
    }
}

impl<T: ActixStream> ActixStream for TlsStream<T> {
    fn poll_read_ready(&self, cx: &mut Context<'_>) -> Poll<io::Result<Ready>> {
        T::poll_read_ready((&**self).get_ref().0, cx)
    }

    fn poll_write_ready(&self, cx: &mut Context<'_>) -> Poll<io::Result<Ready>> {
        T::poll_write_ready((&**self).get_ref().0, cx)
    }
}

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

impl<T: ActixStream> ServiceFactory<T> for Acceptor {
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

impl<T: ActixStream> Service<T> for AcceptorService {
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

pub struct AcceptorServiceFut<T: ActixStream> {
    fut: Accept<T>,
    _guard: CounterGuard,
}

impl<T: ActixStream> Future for AcceptorServiceFut<T> {
    type Output = Result<TlsStream<T>, io::Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        Pin::new(&mut this.fut).poll(cx).map_ok(TlsStream)
    }
}
