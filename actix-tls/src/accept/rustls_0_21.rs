//! `rustls` v0.21 based TLS connection acceptor service.
//!
//! See [`Acceptor`] for main service factory docs.

use std::{
    convert::Infallible,
    future::Future,
    io::{self, IoSlice},
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

use actix_rt::{
    net::{ActixStream, Ready},
    time::{sleep, Sleep},
};
use actix_service::{Service, ServiceFactory};
use actix_utils::{
    counter::{Counter, CounterGuard},
    future::{ready, Ready as FutReady},
};
use pin_project_lite::pin_project;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_rustls::{Accept, TlsAcceptor};
use tokio_rustls_024 as tokio_rustls;

use super::{TlsError, DEFAULT_TLS_HANDSHAKE_TIMEOUT, MAX_CONN_COUNTER};

pub mod reexports {
    //! Re-exports from `rustls` that are useful for acceptors.

    pub use tokio_rustls_024::rustls::ServerConfig;
}

/// Wraps a `rustls` based async TLS stream in order to implement [`ActixStream`].
pub struct TlsStream<IO>(tokio_rustls::server::TlsStream<IO>);

impl_more::impl_from!(<IO> in tokio_rustls::server::TlsStream<IO> => TlsStream<IO>);
impl_more::impl_deref_and_mut!(<IO> in TlsStream<IO> => tokio_rustls::server::TlsStream<IO>);

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
        (**self).is_write_vectored()
    }
}

impl<IO: ActixStream> ActixStream for TlsStream<IO> {
    fn poll_read_ready(&self, cx: &mut Context<'_>) -> Poll<io::Result<Ready>> {
        IO::poll_read_ready((**self).get_ref().0, cx)
    }

    fn poll_write_ready(&self, cx: &mut Context<'_>) -> Poll<io::Result<Ready>> {
        IO::poll_write_ready((**self).get_ref().0, cx)
    }
}

/// Accept TLS connections via the `rustls` crate.
pub struct Acceptor {
    config: Arc<reexports::ServerConfig>,
    handshake_timeout: Duration,
}

impl Acceptor {
    /// Constructs `rustls` based acceptor service factory.
    pub fn new(config: reexports::ServerConfig) -> Self {
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
    type Future = FutReady<Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: ()) -> Self::Future {
        let res = MAX_CONN_COUNTER.with(|conns| {
            Ok(AcceptorService {
                acceptor: self.config.clone().into(),
                conns: conns.clone(),
                handshake_timeout: self.handshake_timeout,
            })
        });

        ready(res)
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
