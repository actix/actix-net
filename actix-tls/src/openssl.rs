use std::future::Future;
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

impl<T: AsyncRead + AsyncWrite + Unpin + 'static> ServiceFactory<T> for Acceptor {
    type Response = SslStream<T>;
    type Error = Error;
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

impl<Req: AsyncRead + AsyncWrite + Unpin + 'static> Service<Req> for AcceptorService {
    type Response = SslStream<Req>;
    type Error = Error;
    type Future = AcceptorServiceResponse<Req>;

    fn poll_ready(&mut self, ctx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        if self.conns.available(ctx) {
            Poll::Ready(Ok(()))
        } else {
            Poll::Pending
        }
    }

    fn call(&mut self, req: Req) -> Self::Future {
        match self.ssl_stream(req) {
            Ok(stream) => {
                let guard = self.conns.get();
                AcceptorServiceResponse::Accept(Some(stream), Some(guard))
            }
            Err(e) => AcceptorServiceResponse::Error(Some(e)),
        }
    }
}

impl AcceptorService {
    // construct a new SslStream.
    // At this point the SslStream does not perform any IO.
    // The handshake would happen later in AcceptorServiceResponse
    fn ssl_stream<Req: AsyncRead + AsyncWrite + Unpin + 'static>(
        &self,
        stream: Req,
    ) -> Result<SslStream<Req>, Error> {
        let ssl = Ssl::new(self.acceptor.context())?;
        let stream = SslStream::new(ssl, stream)?;
        Ok(stream)
    }
}

pub enum AcceptorServiceResponse<Req>
where
    Req: AsyncRead + AsyncWrite,
{
    Accept(Option<SslStream<Req>>, Option<CounterGuard>),
    Error(Option<Error>),
}

impl<Req: AsyncRead + AsyncWrite + Unpin> Future for AcceptorServiceResponse<Req>
where
    Req: AsyncRead + AsyncWrite + Unpin,
{
    type Output = Result<SslStream<Req>, Error>;

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
