use std::{
    convert::TryFrom,
    future::Future,
    io,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

pub use tokio_rustls::{client::TlsStream, rustls::ClientConfig};
pub use webpki_roots::TLS_SERVER_ROOTS;

use actix_rt::net::ActixStream;
use actix_service::{Service, ServiceFactory};
use futures_core::{future::LocalBoxFuture, ready};
use log::trace;
use tokio_rustls::rustls::{client::ServerName, OwnedTrustAnchor, RootCertStore};
use tokio_rustls::{Connect as RustlsConnect, TlsConnector as RustlsTlsConnector};

use crate::connect::{Address, Connection};

/// Returns standard root certificates from `webpki-roots` crate as a rustls certificate store.
pub fn webpki_roots_cert_store() -> RootCertStore {
    let mut root_certs = RootCertStore::empty();
    for cert in TLS_SERVER_ROOTS.0 {
        let cert = OwnedTrustAnchor::from_subject_spki_name_constraints(
            cert.subject,
            cert.spki,
            cert.name_constraints,
        );
        let certs = vec![cert].into_iter();
        root_certs.add_server_trust_anchors(certs);
    }
    root_certs
}

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

impl<R, IO> ServiceFactory<Connection<R, IO>> for RustlsConnector
where
    R: Address,
    IO: ActixStream + 'static,
{
    type Response = Connection<R, TlsStream<IO>>;
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

impl<R, IO> Service<Connection<R, IO>> for RustlsConnectorService
where
    R: Address,
    IO: ActixStream,
{
    type Response = Connection<R, TlsStream<IO>>;
    type Error = io::Error;
    type Future = RustlsConnectorServiceFuture<R, IO>;

    actix_service::always_ready!();

    fn call(&self, connection: Connection<R, IO>) -> Self::Future {
        trace!("SSL Handshake start for: {:?}", connection.host());
        let (stream, connection) = connection.replace_io(());

        match ServerName::try_from(connection.host()) {
            Ok(host) => RustlsConnectorServiceFuture::Future {
                connect: RustlsTlsConnector::from(self.connector.clone()).connect(host, stream),
                connection: Some(connection),
            },
            Err(_) => RustlsConnectorServiceFuture::InvalidDns,
        }
    }
}

pub enum RustlsConnectorServiceFuture<R, IO> {
    /// See issue <https://github.com/briansmith/webpki/issues/54>
    InvalidDns,
    Future {
        connect: RustlsConnect<IO>,
        connection: Option<Connection<R, ()>>,
    },
}

impl<R, IO> Future for RustlsConnectorServiceFuture<R, IO>
where
    R: Address,
    IO: ActixStream,
{
    type Output = Result<Connection<R, TlsStream<IO>>, io::Error>;

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
