use std::task::{Context, Poll};

use actix_codec::{AsyncRead, AsyncWrite};
use actix_service::{Service, ServiceFactory};
use actix_utils::counter::Counter;
use futures_core::future::LocalBoxFuture;

pub use tokio_native_tls::native_tls::Error;
pub use tokio_native_tls::{TlsAcceptor, TlsStream};

use super::MAX_CONN_COUNTER;

/// Accept TLS connections via `native-tls` package.
///
/// `native-tls` feature enables this `Acceptor` type.
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

impl<T> ServiceFactory<T> for Acceptor
where
    T: AsyncRead + AsyncWrite + Unpin + 'static,
{
    type Response = TlsStream<T>;
    type Error = Error;
    type Config = ();

    type Service = NativeTlsAcceptorService;
    type InitError = ();
    type Future = LocalBoxFuture<'static, Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: ()) -> Self::Future {
        let res = MAX_CONN_COUNTER.with(|conns| {
            Ok(NativeTlsAcceptorService {
                acceptor: self.acceptor.clone(),
                conns: conns.clone(),
            })
        });
        Box::pin(async { res })
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

impl<T> Service<T> for NativeTlsAcceptorService
where
    T: AsyncRead + AsyncWrite + Unpin + 'static,
{
    type Response = TlsStream<T>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<TlsStream<T>, Error>>;

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        if self.conns.available(cx) {
            Poll::Ready(Ok(()))
        } else {
            Poll::Pending
        }
    }

    fn call(&self, io: T) -> Self::Future {
        let guard = self.conns.get();
        let this = self.clone();
        Box::pin(async move {
            let io = this.acceptor.accept(io).await;
            drop(guard);
            io
        })
    }
}
