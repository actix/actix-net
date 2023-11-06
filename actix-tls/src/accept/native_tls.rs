//! `native-tls` based TLS connection acceptor service.
//!
//! See [`Acceptor`] for main service factory docs.

use std::{
    convert::Infallible,
    io::{self, IoSlice},
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use actix_rt::{
    net::{ActixStream, Ready},
    time::timeout,
};
use actix_service::{Service, ServiceFactory};
use actix_utils::{
    counter::Counter,
    future::{ready, Ready as FutReady},
};
use futures_core::future::LocalBoxFuture;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_native_tls::{native_tls::Error, TlsAcceptor};

use super::{TlsError, DEFAULT_TLS_HANDSHAKE_TIMEOUT, MAX_CONN_COUNTER};

pub mod reexports {
    //! Re-exports from `native-tls` that are useful for acceptors.

    pub use tokio_native_tls::{native_tls::Error, TlsAcceptor};
}

/// Wraps a `native-tls` based async TLS stream in order to implement [`ActixStream`].
pub struct TlsStream<IO>(tokio_native_tls::TlsStream<IO>);

impl_more::impl_from!(<IO> in tokio_native_tls::TlsStream<IO> => TlsStream<IO>);
impl_more::impl_deref_and_mut!(<IO> in TlsStream<IO> => tokio_native_tls::TlsStream<IO>);

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
        IO::poll_read_ready((**self).get_ref().get_ref().get_ref(), cx)
    }

    fn poll_write_ready(&self, cx: &mut Context<'_>) -> Poll<io::Result<Ready>> {
        IO::poll_write_ready((**self).get_ref().get_ref().get_ref(), cx)
    }
}

/// Accept TLS connections via the `native-tls` crate.
pub struct Acceptor {
    acceptor: TlsAcceptor,
    handshake_timeout: Duration,
}

impl Acceptor {
    /// Constructs `native-tls` based acceptor service factory.
    pub fn new(acceptor: TlsAcceptor) -> Self {
        Acceptor {
            acceptor,
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
            acceptor: self.acceptor.clone(),
            handshake_timeout: self.handshake_timeout,
        }
    }
}

impl<IO: ActixStream + 'static> ServiceFactory<IO> for Acceptor {
    type Response = TlsStream<IO>;
    type Error = TlsError<Error, Infallible>;
    type Config = ();
    type Service = AcceptorService;
    type InitError = ();
    type Future = FutReady<Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: ()) -> Self::Future {
        let res = MAX_CONN_COUNTER.with(|conns| {
            Ok(AcceptorService {
                acceptor: self.acceptor.clone(),
                conns: conns.clone(),
                handshake_timeout: self.handshake_timeout,
            })
        });

        ready(res)
    }
}

/// Native-TLS based acceptor service.
pub struct AcceptorService {
    acceptor: TlsAcceptor,
    conns: Counter,
    handshake_timeout: Duration,
}

impl<IO: ActixStream + 'static> Service<IO> for AcceptorService {
    type Response = TlsStream<IO>;
    type Error = TlsError<Error, Infallible>;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        if self.conns.available(cx) {
            Poll::Ready(Ok(()))
        } else {
            Poll::Pending
        }
    }

    fn call(&self, io: IO) -> Self::Future {
        let guard = self.conns.get();
        let acceptor = self.acceptor.clone();

        let dur = self.handshake_timeout;

        Box::pin(async move {
            match timeout(dur, acceptor.accept(io)).await {
                Ok(Ok(io)) => {
                    drop(guard);
                    Ok(TlsStream(io))
                }
                Ok(Err(err)) => Err(TlsError::Tls(err)),
                Err(_timeout) => Err(TlsError::Timeout),
            }
        })
    }
}
