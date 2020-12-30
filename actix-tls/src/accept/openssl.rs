use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use actix_codec::{AsyncRead, AsyncWrite};
use actix_service::{Service, ServiceFactory};
use actix_utils::counter::{Counter, CounterGuard};
use futures_util::{
    future::{ready, Ready},
    ready,
};

pub use openssl::ssl::{
    AlpnError, Error as SslError, HandshakeError, Ssl, SslAcceptor, SslAcceptorBuilder,
};
pub use tokio_openssl::SslStream;

use super::MAX_CONN_COUNTER;

/// Accept TLS connections via `openssl` package.
///
/// `openssl` feature enables this `Acceptor` type.
pub struct Acceptor {
    acceptor: SslAcceptor,
}

impl Acceptor {
    /// Create OpenSSL based `Acceptor` service factory.
    #[inline]
    pub fn new(acceptor: SslAcceptor) -> Self {
        Acceptor { acceptor }
    }
}

impl Clone for Acceptor {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            acceptor: self.acceptor.clone(),
        }
    }
}

impl<T> ServiceFactory<T> for Acceptor
where
    T: AsyncRead + AsyncWrite + Unpin + 'static,
{
    type Response = SslStream<T>;
    type Error = SslError;
    type Config = ();
    type Service = AcceptorService;
    type InitError = ();
    type Future = Ready<Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: ()) -> Self::Future {
        MAX_CONN_COUNTER.with(|conns| {
            ready(Ok(AcceptorService {
                acceptor: self.acceptor.clone(),
                conns: conns.clone(),
            }))
        })
    }
}

pub struct AcceptorService {
    acceptor: SslAcceptor,
    conns: Counter,
}

impl<T> Service<T> for AcceptorService
where
    T: AsyncRead + AsyncWrite + Unpin + 'static,
{
    type Response = SslStream<T>;
    type Error = SslError;
    type Future = AcceptorServiceResponse<T>;

    fn poll_ready(&mut self, ctx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        if self.conns.available(ctx) {
            Poll::Ready(Ok(()))
        } else {
            Poll::Pending
        }
    }

    fn call(&mut self, io: T) -> Self::Future {
        let ssl_ctx = self.acceptor.context();
        let ssl = Ssl::new(ssl_ctx).expect("Provided SSL acceptor was invalid.");
        AcceptorServiceResponse {
            _guard: self.conns.get(),
            stream: Some(SslStream::new(ssl, io).unwrap()),
        }
    }
}

pub struct AcceptorServiceResponse<T>
where
    T: AsyncRead + AsyncWrite,
{
    stream: Option<SslStream<T>>,
    _guard: CounterGuard,
}

impl<T: AsyncRead + AsyncWrite + Unpin> Future for AcceptorServiceResponse<T> {
    type Output = Result<SslStream<T>, SslError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        ready!(Pin::new(self.stream.as_mut().unwrap()).poll_accept(cx))?;
        Poll::Ready(Ok(self.stream.take().expect("SSL connect has resolved.")))
    }
}
