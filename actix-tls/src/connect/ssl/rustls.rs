use std::{
    fmt,
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

pub use rustls::Session;
pub use tokio_rustls::{client::TlsStream, rustls::ClientConfig};
pub use webpki_roots::TLS_SERVER_ROOTS;

use actix_codec::{AsyncRead, AsyncWrite};
use actix_service::{Service, ServiceFactory};
use futures_core::{future::LocalBoxFuture, ready};
use log::trace;
use tokio_rustls::{Connect, TlsConnector};
use webpki::DNSNameRef;

use crate::connect::{Address, Connection};

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
    type Future = LocalBoxFuture<'static, Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: ()) -> Self::Future {
        let connector = self.connector.clone();
        Box::pin(async { Ok(RustlsConnectorService { connector }) })
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

    fn call(&self, stream: Connection<T, U>) -> Self::Future {
        trace!("SSL Handshake start for: {:?}", stream.host());
        let (io, stream) = stream.replace_io(());
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

impl<T, U> Future for ConnectAsyncExt<T, U>
where
    T: Address,
    U: AsyncRead + AsyncWrite + Unpin + fmt::Debug,
{
    type Output = Result<Connection<T, TlsStream<U>>, std::io::Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let stream = ready!(Pin::new(&mut this.fut).poll(cx))?;
        let s = this.stream.take().unwrap();
        trace!("SSL Handshake success: {:?}", s.host());
        Poll::Ready(Ok(s.replace_io(stream).1))
    }
}
