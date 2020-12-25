use std::future::Future;
use std::marker::PhantomData;
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
pub struct Acceptor<T> {
    acceptor: TlsAcceptor,
    io: PhantomData<T>,
}

impl<T> Acceptor<T>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    /// Create `native-tls` based `Acceptor` service factory.
    #[inline]
    pub fn new(acceptor: TlsAcceptor) -> Self {
        Acceptor {
            acceptor,
            io: PhantomData,
        }
    }
}

impl<T> Clone for Acceptor<T> {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            acceptor: self.acceptor.clone(),
            io: PhantomData,
        }
    }
}

impl<T> ServiceFactory for Acceptor<T>
where
    T: AsyncRead + AsyncWrite + Unpin + 'static,
{
    type Request = T;
    type Response = TlsStream<T>;
    type Error = Error;
    type Config = ();

    type Service = NativeTlsAcceptorService<T>;
    type InitError = ();
    type Future = Ready<Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: ()) -> Self::Future {
        MAX_CONN_COUNTER.with(|conns| {
            ready(Ok(NativeTlsAcceptorService {
                acceptor: self.acceptor.clone(),
                conns: conns.clone(),
                io: PhantomData,
            }))
        })
    }
}

pub struct NativeTlsAcceptorService<T> {
    acceptor: TlsAcceptor,
    io: PhantomData<T>,
    conns: Counter,
}

impl<T> Clone for NativeTlsAcceptorService<T> {
    fn clone(&self) -> Self {
        Self {
            acceptor: self.acceptor.clone(),
            io: PhantomData,
            conns: self.conns.clone(),
        }
    }
}

type LocalBoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

impl<T> Service for NativeTlsAcceptorService<T>
where
    T: AsyncRead + AsyncWrite + Unpin + 'static,
{
    type Request = T;
    type Response = TlsStream<T>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<TlsStream<T>, Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        if self.conns.available(cx) {
            Poll::Ready(Ok(()))
        } else {
            Poll::Pending
        }
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        let guard = self.conns.get();
        let this = self.clone();
        Box::pin(async move {
            let res = this.acceptor.accept(req).await;
            drop(guard);
            res
        })
    }
}
