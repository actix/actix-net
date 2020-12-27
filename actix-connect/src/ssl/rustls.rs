use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

pub use rust_tls::Session;
pub use tokio_rustls::{client::TlsStream, rustls::ClientConfig};

use actix_codec::{AsyncRead, AsyncWrite};
use actix_service::{Service, ServiceFactory};
use futures_util::future::{ready, Ready};
use futures_util::ready;
use tokio_rustls::{Connect, TlsConnector};
use webpki::DNSNameRef;

use crate::{Address, Connection};

/// Rustls connector factory
pub struct RustlsConnector {
    connector: Arc<ClientConfig>,
}

impl RustlsConnector {
    pub fn new(connector: Arc<ClientConfig>) -> Self {
        RustlsConnector { connector }
    }
}

impl RustlsConnector {
    pub fn service(connector: Arc<ClientConfig>) -> RustlsConnectorService {
        RustlsConnectorService { connector }
    }
}

impl Clone for RustlsConnector {
    fn clone(&self) -> Self {
        Self {
            connector: self.connector.clone(),
        }
    }
}

impl<T: Address, U> ServiceFactory<Connection<T, U>> for RustlsConnector
where
    U: AsyncRead + AsyncWrite + Unpin + fmt::Debug,
{
    type Response = Connection<T, TlsStream<U>>;
    type Error = std::io::Error;
    type Config = ();
    type Service = RustlsConnectorService;
    type InitError = ();
    type Future = Ready<Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: ()) -> Self::Future {
        ready(Ok(RustlsConnectorService {
            connector: self.connector.clone(),
        }))
    }
}

pub struct RustlsConnectorService {
    connector: Arc<ClientConfig>,
}

impl Clone for RustlsConnectorService {
    fn clone(&self) -> Self {
        Self {
            connector: self.connector.clone(),
        }
    }
}

impl<T, U> Service<Connection<T, U>> for RustlsConnectorService
where
    T: Address,
    U: AsyncRead + AsyncWrite + Unpin + fmt::Debug,
{
    type Response = Connection<T, TlsStream<U>>;
    type Error = std::io::Error;
    type Future = ConnectAsyncExt<T, U>;

    actix_service::always_ready!();

    fn call(&mut self, stream: Connection<T, U>) -> Self::Future {
        trace!("SSL Handshake start for: {:?}", stream.host());
        let (io, stream) = stream.replace(());
        let host = DNSNameRef::try_from_ascii_str(stream.host())
            .expect("rustls currently only handles hostname-based connections. See https://github.com/briansmith/webpki/issues/54");
        ConnectAsyncExt {
            fut: TlsConnector::from(self.connector.clone()).connect(host, io),
            stream: Some(stream),
        }
    }
}

pub struct ConnectAsyncExt<T, U> {
    fut: Connect<U>,
    stream: Option<Connection<T, ()>>,
}

impl<T: Address, U> Future for ConnectAsyncExt<T, U>
where
    U: AsyncRead + AsyncWrite + Unpin + fmt::Debug,
{
    type Output = Result<Connection<T, TlsStream<U>>, std::io::Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let stream = ready!(Pin::new(&mut this.fut).poll(cx))?;
        let s = this.stream.take().unwrap();
        trace!("SSL Handshake success: {:?}", s.host());
        Poll::Ready(Ok(s.replace(stream).1))
    }
}
