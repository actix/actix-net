use std::{
    future::Future,
    io,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

pub use tokio_rustls::rustls::Session;
pub use tokio_rustls::{client::TlsStream, rustls::ClientConfig};
pub use webpki_roots::TLS_SERVER_ROOTS;

use actix_rt::net::ActixStream;
use actix_service::{Service, ServiceFactory};
use futures_core::{future::LocalBoxFuture, ready};
use log::trace;
use tokio_rustls::webpki::DNSNameRef;
use tokio_rustls::{Connect, TlsConnector};

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

impl<T, U> ServiceFactory<Connection<T, U>> for RustlsConnector
where
    T: Address,
    U: ActixStream + 'static,
{
    type Response = Connection<T, TlsStream<U>>;
    type Error = io::Error;
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
    U: ActixStream,
{
    type Response = Connection<T, TlsStream<U>>;
    type Error = io::Error;
    type Future = RustlsConnectorServiceFuture<T, U>;

    actix_service::always_ready!();

    fn call(&self, connection: Connection<T, U>) -> Self::Future {
        trace!("SSL Handshake start for: {:?}", connection.host());
        let (stream, connection) = connection.replace_io(());

        match DNSNameRef::try_from_ascii_str(connection.host()) {
            Ok(host) => RustlsConnectorServiceFuture::Future {
                connect: TlsConnector::from(self.connector.clone()).connect(host, stream),
                connection: Some(connection),
            },
            Err(_) => RustlsConnectorServiceFuture::InvalidDns,
        }
    }
}

pub enum RustlsConnectorServiceFuture<T, U> {
    /// See issue https://github.com/briansmith/webpki/issues/54
    InvalidDns,
    Future {
        connect: Connect<U>,
        connection: Option<Connection<T, ()>>,
    },
}

impl<T, U> Future for RustlsConnectorServiceFuture<T, U>
where
    T: Address,
    U: ActixStream,
{
    type Output = Result<Connection<T, TlsStream<U>>, io::Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.get_mut() {
            Self::InvalidDns => Poll::Ready(Err(
                io::Error::new(io::ErrorKind::Other, "rustls currently only handles hostname-based connections. See https://github.com/briansmith/webpki/issues/54")
            )),
            Self::Future { connect, connection } => {
                let stream = ready!(Pin::new(connect).poll(cx))?;
                let connection = connection.take().unwrap();
                trace!("SSL Handshake success: {:?}", connection.host());
                Poll::Ready(Ok(connection.replace_io(stream).1))
            }
        }
    }
}
