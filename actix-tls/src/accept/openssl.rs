//! `openssl` based TLS acceptor service.
//!
//! See [`Acceptor`] for main service factory docs.

use std::{
    convert::Infallible,
    future::Future,
    io::{self, IoSlice},
    pin::Pin,
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
use openssl::ssl::{Error, Ssl, SslAcceptor};
use pin_project_lite::pin_project;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use super::{TlsError, DEFAULT_TLS_HANDSHAKE_TIMEOUT, MAX_CONN_COUNTER};

pub mod reexports {
    //! Re-exports from `openssl` that are useful for acceptors.

    pub use openssl::ssl::{
        AlpnError, Error, HandshakeError, Ssl, SslAcceptor, SslAcceptorBuilder,
    };
}

/// Wraps an `openssl` based async TLS stream in order to implement [`ActixStream`].
pub struct TlsStream<IO>(tokio_openssl::SslStream<IO>);

impl_more::impl_from!(<IO> in tokio_openssl::SslStream<IO> => TlsStream<IO>);
impl_more::impl_deref_and_mut!(<IO> in TlsStream<IO> => tokio_openssl::SslStream<IO>);

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
        IO::poll_read_ready((**self).get_ref(), cx)
    }

    fn poll_write_ready(&self, cx: &mut Context<'_>) -> Poll<io::Result<Ready>> {
        IO::poll_write_ready((**self).get_ref(), cx)
    }
}

/// Accept TLS connections via the `openssl` crate.
pub struct Acceptor {
    acceptor: SslAcceptor,
    handshake_timeout: Duration,
}

impl Acceptor {
    /// Create `openssl` based acceptor service factory.
    #[inline]
    pub fn new(acceptor: SslAcceptor) -> Self {
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

impl<IO: ActixStream> ServiceFactory<IO> for Acceptor {
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

/// OpenSSL based acceptor service.
pub struct AcceptorService {
    acceptor: SslAcceptor,
    conns: Counter,
    handshake_timeout: Duration,
}

impl<IO: ActixStream> Service<IO> for AcceptorService {
    type Response = TlsStream<IO>;
    type Error = TlsError<Error, Infallible>;
    type Future = AcceptFut<IO>;

    fn poll_ready(&self, ctx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        if self.conns.available(ctx) {
            Poll::Ready(Ok(()))
        } else {
            Poll::Pending
        }
    }

    fn call(&self, io: IO) -> Self::Future {
        let ssl_ctx = self.acceptor.context();
        let ssl = Ssl::new(ssl_ctx).expect("Provided SSL acceptor was invalid.");

        AcceptFut {
            _guard: self.conns.get(),
            timeout: sleep(self.handshake_timeout),
            stream: Some(tokio_openssl::SslStream::new(ssl, io).unwrap()),
        }
    }
}

pin_project! {
    /// Accept future for OpenSSL service.
    #[doc(hidden)]
    pub struct AcceptFut<IO: ActixStream> {
        stream: Option<tokio_openssl::SslStream<IO>>,
        #[pin]
        timeout: Sleep,
        _guard: CounterGuard,
    }
}

impl<IO: ActixStream> Future for AcceptFut<IO> {
    type Output = Result<TlsStream<IO>, TlsError<Error, Infallible>>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        match Pin::new(this.stream.as_mut().unwrap()).poll_accept(cx) {
            Poll::Ready(Ok(())) => Poll::Ready(Ok(this
                .stream
                .take()
                .expect("Acceptor should not be polled after it has completed.")
                .into())),
            Poll::Ready(Err(err)) => Poll::Ready(Err(TlsError::Tls(err))),
            Poll::Pending => this.timeout.poll(cx).map(|_| Err(TlsError::Timeout)),
        }
    }
}
