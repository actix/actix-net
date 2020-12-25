use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};

use actix_codec::{AsyncRead, AsyncWrite};
use actix_service::{Service, ServiceFactory};
use actix_utils::counter::{Counter, CounterGuard};
use futures_util::future::{ready, Ready};
use futures_util::ready;

pub use open_ssl::ssl::{AlpnError, Error, Ssl, SslAcceptor, SslAcceptorBuilder};
pub use tokio_openssl::SslStream;

use crate::MAX_CONN_COUNTER;

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

impl<T: AsyncRead + AsyncWrite + Unpin + 'static> ServiceFactory for Acceptor<T> {
    type Request = T;
    type Response = SslStream<T>;
    type Error = Error;
    type Config = ();
    type Service = AcceptorService<T>;
    type InitError = ();
    type Future = Ready<Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: ()) -> Self::Future {
        MAX_CONN_COUNTER.with(|conns| {
            ready(Ok(AcceptorService {
                acceptor: self.acceptor.clone(),
                conns: conns.clone(),
                io: PhantomData,
            }))
        })
    }
}

pub struct AcceptorService<T> {
    acceptor: SslAcceptor,
    conns: Counter,
    io: PhantomData<T>,
}

impl<T: AsyncRead + AsyncWrite + Unpin + 'static> Service for AcceptorService<T> {
    type Request = T;
    type Response = SslStream<T>;
    type Error = Error;
    type Future = AcceptorServiceResponse<T>;

    fn poll_ready(&mut self, ctx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        if self.conns.available(ctx) {
            Poll::Ready(Ok(()))
        } else {
            Poll::Pending
        }
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        match self.ssl_stream(req) {
            Ok(stream) => {
                let guard = self.conns.get();
                AcceptorServiceResponse::Accept(Some(stream), Some(guard))
            }
            Err(e) => AcceptorServiceResponse::Error(Some(e)),
        }
    }
}

impl<T: AsyncRead + AsyncWrite + Unpin + 'static> AcceptorService<T> {
    // construct a new SslStream.
    // At this point the SslStream does not perform any IO.
    // The handshake would happen later in AcceptorServiceResponse
    fn ssl_stream(&self, stream: T) -> Result<SslStream<T>, Error> {
        let ssl = Ssl::new(self.acceptor.context())?;
        let stream = SslStream::new(ssl, stream)?;
        Ok(stream)
    }
}

pub enum AcceptorServiceResponse<T>
where
    T: AsyncRead + AsyncWrite,
{
    Accept(Option<SslStream<T>>, Option<CounterGuard>),
    Error(Option<Error>),
}

impl<T: AsyncRead + AsyncWrite + Unpin> Future for AcceptorServiceResponse<T> {
    type Output = Result<SslStream<T>, Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.get_mut() {
            AcceptorServiceResponse::Error(e) => Poll::Ready(Err(e.take().unwrap())),
            AcceptorServiceResponse::Accept(stream, guard) => {
                ready!(Pin::new(stream.as_mut().unwrap()).poll_accept(cx))?;
                // drop counter guard a little early as the accept has finished
                guard.take();
                Poll::Ready(Ok(stream.take().unwrap()))
            }
        }
    }
}
