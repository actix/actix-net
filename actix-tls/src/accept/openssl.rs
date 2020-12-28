use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};

use actix_codec::{AsyncRead, AsyncWrite};
use actix_service::{Service, ServiceFactory};
use actix_utils::counter::{Counter, CounterGuard};
use futures_util::{
    future::{ok, Ready},
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
pub struct Acceptor<T: AsyncRead + AsyncWrite> {
    acceptor: SslAcceptor,
    io: PhantomData<T>,
}

impl<T: AsyncRead + AsyncWrite> Acceptor<T> {
    /// Create OpenSSL based `Acceptor` service factory.
    #[inline]
    pub fn new(acceptor: SslAcceptor) -> Self {
        Acceptor {
            acceptor,
            io: PhantomData,
        }
    }
}

impl<T: AsyncRead + AsyncWrite> Clone for Acceptor<T> {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            acceptor: self.acceptor.clone(),
            io: PhantomData,
        }
    }
}

impl<T> ServiceFactory<T> for Acceptor<T>
where
    T: AsyncRead + AsyncWrite + Unpin + 'static,
{
    type Response = SslStream<T>;
    type Error = SslError;
    type Config = ();
    type Service = AcceptorService<T>;
    type InitError = ();
    type Future = Ready<Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: ()) -> Self::Future {
        MAX_CONN_COUNTER.with(|conns| {
            ok(AcceptorService {
                acceptor: self.acceptor.clone(),
                conns: conns.clone(),
                io: PhantomData,
            })
        })
    }
}

pub struct AcceptorService<T> {
    acceptor: SslAcceptor,
    conns: Counter,
    io: PhantomData<T>,
}

impl<T> Service<T> for AcceptorService<T>
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
        let acc = self.acceptor.clone();
        let ssl_ctx = acc.into_context();
        let ssl = Ssl::new(&ssl_ctx).expect("Provided SSL acceptor was invalid.");

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
        ready!(Pin::new(self.stream.as_mut().unwrap()).poll_connect(cx))?;
        Poll::Ready(Ok(self.stream.take().expect("SSL connect has resolved.")))
    }
}
