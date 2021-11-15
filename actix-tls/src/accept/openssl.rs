use std::{
    convert::Infallible,
    future::Future,
    io::{self, IoSlice},
    ops::{Deref, DerefMut},
    pin::Pin,
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

pub use openssl::ssl::{
    AlpnError, Error as SslError, HandshakeError, Ssl, SslAcceptor, SslAcceptorBuilder,
};
use pin_project_lite::pin_project;

use super::{TlsError, DEFAULT_TLS_HANDSHAKE_TIMEOUT, MAX_CONN_COUNTER};

/// Wrapper type for `tokio_openssl::SslStream` in order to impl `ActixStream` trait.
pub struct TlsStream<T>(tokio_openssl::SslStream<T>);

impl<T> From<tokio_openssl::SslStream<T>> for TlsStream<T> {
    fn from(stream: tokio_openssl::SslStream<T>) -> Self {
        Self(stream)
    }
}

impl<T> Deref for TlsStream<T> {
    type Target = tokio_openssl::SslStream<T>;

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
        T::poll_read_ready((&**self).get_ref(), cx)
    }

    fn poll_write_ready(&self, cx: &mut Context<'_>) -> Poll<io::Result<Ready>> {
        T::poll_write_ready((&**self).get_ref(), cx)
    }
}

/// Accept TLS connections via `openssl` package.
///
/// `openssl` feature enables this `Acceptor` type.
pub struct Acceptor {
    acceptor: SslAcceptor,
    handshake_timeout: Duration,
}

impl Acceptor {
    /// Create OpenSSL based `Acceptor` service factory.
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

impl<T: ActixStream> ServiceFactory<T> for Acceptor {
    type Response = TlsStream<T>;
    type Error = TlsError<SslError, Infallible>;
    type Config = ();
    type Service = AcceptorService;
    type InitError = ();
    type Future = LocalBoxFuture<'static, Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: ()) -> Self::Future {
        let res = MAX_CONN_COUNTER.with(|conns| {
            Ok(AcceptorService {
                acceptor: self.acceptor.clone(),
                conns: conns.clone(),
                handshake_timeout: self.handshake_timeout,
            })
        });

        Box::pin(async { res })
    }
}

pub struct AcceptorService {
    acceptor: SslAcceptor,
    conns: Counter,
    handshake_timeout: Duration,
}

impl<T: ActixStream> Service<T> for AcceptorService {
    type Response = TlsStream<T>;
    type Error = TlsError<SslError, Infallible>;
    type Future = AcceptorServiceResponse<T>;

    fn poll_ready(&self, ctx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        if self.conns.available(ctx) {
            Poll::Ready(Ok(()))
        } else {
            Poll::Pending
        }
    }

    fn call(&self, io: T) -> Self::Future {
        let ssl_ctx = self.acceptor.context();
        let ssl = Ssl::new(ssl_ctx).expect("Provided SSL acceptor was invalid.");

        AcceptorServiceResponse {
            _guard: self.conns.get(),
            timeout: sleep(self.handshake_timeout),
            stream: Some(tokio_openssl::SslStream::new(ssl, io).unwrap()),
        }
    }
}

pin_project! {
    pub struct AcceptorServiceResponse<T: ActixStream> {
        stream: Option<tokio_openssl::SslStream<T>>,
        #[pin]
        timeout: Sleep,
        _guard: CounterGuard,
    }
}

impl<T: ActixStream> Future for AcceptorServiceResponse<T> {
    type Output = Result<TlsStream<T>, TlsError<SslError, Infallible>>;

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
