//! Rustls based acceptor service.

use std::{
    convert::Infallible,
    future::Future,
    io::{self, IoSlice},
    ops::{Deref, DerefMut},
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

use actix_codec::{AsyncRead, AsyncWrite, ReadBuf};
use actix_rt::{
    net::{ActixStream, Ready},
    time::{sleep, Sleep},
};
use actix_service::{Service, ServiceFactory};
use actix_utils::counter::{Counter, CounterGuard};
use futures_core::future::LocalBoxFuture;
use pin_project_lite::pin_project;
pub use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::{Accept, TlsAcceptor};

use super::{TlsError, DEFAULT_TLS_HANDSHAKE_TIMEOUT, MAX_CONN_COUNTER};

/// Wraps a [`tokio_rustls::server::TlsStream`] in order to impl [`ActixStream`] trait.
pub struct TlsStream<IO>(tokio_rustls::server::TlsStream<IO>);

impl<IO> From<tokio_rustls::server::TlsStream<IO>> for TlsStream<IO> {
    fn from(stream: tokio_rustls::server::TlsStream<IO>) -> Self {
        Self(stream)
    }
}

impl<IO> Deref for TlsStream<IO> {
    type Target = tokio_rustls::server::TlsStream<IO>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<IO> DerefMut for TlsStream<IO> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<IO: ActixStream> AsyncRead for TlsStream<IO> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut **self.get_mut()).poll_read(cx, buf)
    }
}

impl<IO: ActixStream> AsyncWrite for TlsStream<IO> {
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

impl<IO: ActixStream> ActixStream for TlsStream<IO> {
    fn poll_read_ready(&self, cx: &mut Context<'_>) -> Poll<io::Result<Ready>> {
        IO::poll_read_ready((&**self).get_ref().0, cx)
    }

    fn poll_write_ready(&self, cx: &mut Context<'_>) -> Poll<io::Result<Ready>> {
        IO::poll_write_ready((&**self).get_ref().0, cx)
    }
}

/// Accept TLS connections via `rustls` package.
///
/// `rustls` feature enables this `Acceptor` type.
pub struct Acceptor {
    config: Arc<ServerConfig>,
    handshake_timeout: Duration,
}

impl Acceptor {
    /// Create Rustls based `Acceptor` service factory.
    #[inline]
    pub fn new(config: ServerConfig) -> Self {
        Acceptor {
            config: Arc::new(config),
            handshake_timeout: DEFAULT_TLS_HANDSHAKE_TIMEOUT,
        }
    }

    /// Limit the amount of time that the acceptor will wait for a TLS handshake to complete.
    ///
    /// Default timeout is 3 seconds.
    pub fn set_handshake_timeout(&mut self, handshake_timeout: Duration) -> &mut Self {
        self.handshake_timeout = handshake_timeout;
        self
    }
}

impl Clone for Acceptor {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            handshake_timeout: self.handshake_timeout,
        }
    }
}

impl<IO: ActixStream> ServiceFactory<IO> for Acceptor {
    type Response = TlsStream<IO>;
    type Error = TlsError<io::Error, Infallible>;
    type Config = ();
    type Service = AcceptorService;
    type InitError = ();
    type Future = LocalBoxFuture<'static, Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: ()) -> Self::Future {
        let res = MAX_CONN_COUNTER.with(|conns| {
            Ok(AcceptorService {
                acceptor: self.config.clone().into(),
                conns: conns.clone(),
                handshake_timeout: self.handshake_timeout,
            })
        });

        Box::pin(async { res })
    }
}

/// Rustls based acceptor service.
pub struct AcceptorService {
    acceptor: TlsAcceptor,
    conns: Counter,
    handshake_timeout: Duration,
}

impl<IO: ActixStream> Service<IO> for AcceptorService {
    type Response = TlsStream<IO>;
    type Error = TlsError<io::Error, Infallible>;
    type Future = AcceptFut<IO>;

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        if self.conns.available(cx) {
            Poll::Ready(Ok(()))
        } else {
            Poll::Pending
        }
    }

    fn call(&self, req: IO) -> Self::Future {
        AcceptFut {
            fut: self.acceptor.accept(req),
            timeout: sleep(self.handshake_timeout),
            _guard: self.conns.get(),
        }
    }
}

pin_project! {
    /// Accept future for Rustls service.
    #[doc(hidden)]
    pub struct AcceptFut<IO: ActixStream> {
        fut: Accept<IO>,
        #[pin]
        timeout: Sleep,
        _guard: CounterGuard,
    }
}

impl<IO: ActixStream> Future for AcceptFut<IO> {
    type Output = Result<TlsStream<IO>, TlsError<io::Error, Infallible>>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();
        match Pin::new(&mut this.fut).poll(cx) {
            Poll::Ready(Ok(stream)) => Poll::Ready(Ok(TlsStream(stream))),
            Poll::Ready(Err(err)) => Poll::Ready(Err(TlsError::Tls(err))),
            Poll::Pending => this.timeout.poll(cx).map(|_| Err(TlsError::Timeout)),
        }
    }
}
