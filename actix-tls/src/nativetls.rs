use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use actix_codec::{AsyncRead, AsyncWrite};
use actix_service::{Service, ServiceFactory};
use actix_utils::counter::Counter;
use futures_util::future::{ready, Ready};

pub use native_tls::Error;
pub use tokio_native_tls::{TlsAcceptor, TlsStream};

use crate::MAX_CONN_COUNTER;

/// Accept TLS connections via `native-tls` package.
///
/// `nativetls` feature enables this `Acceptor` type.
pub struct Acceptor {
    acceptor: TlsAcceptor,
}

impl Acceptor {
    /// Create `native-tls` based `Acceptor` service factory.
    #[inline]
    pub fn new(acceptor: TlsAcceptor) -> Self {
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

impl<Req> ServiceFactory<Req> for Acceptor
where
    Req: AsyncRead + AsyncWrite + Unpin + 'static,
{
    type Response = TlsStream<Req>;
    type Error = Error;
    type Config = ();

    type Service = NativeTlsAcceptorService;
    type InitError = ();
    type Future = Ready<Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: ()) -> Self::Future {
        MAX_CONN_COUNTER.with(|conns| {
            ready(Ok(NativeTlsAcceptorService {
                acceptor: self.acceptor.clone(),
                conns: conns.clone(),
            }))
        })
    }
}

pub struct NativeTlsAcceptorService {
    acceptor: TlsAcceptor,
    conns: Counter,
}

impl Clone for NativeTlsAcceptorService {
    fn clone(&self) -> Self {
        Self {
            acceptor: self.acceptor.clone(),
            conns: self.conns.clone(),
        }
    }
}

type LocalBoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

impl<Req> Service<Req> for NativeTlsAcceptorService
where
    Req: AsyncRead + AsyncWrite + Unpin + 'static,
{
    type Response = TlsStream<Req>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<TlsStream<Req>, Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        if self.conns.available(cx) {
            Poll::Ready(Ok(()))
        } else {
            Poll::Pending
        }
    }

    fn call(&mut self, req: Req) -> Self::Future {
        let guard = self.conns.get();
        let this = self.clone();
        Box::pin(async move {
            let res = this.acceptor.accept(req).await;
            drop(guard);
            res
        })
    }
}
