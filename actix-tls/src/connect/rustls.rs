//! Rustls based connector service.
//!
//! See [`TlsConnector`] for main connector service factory docs.

use std::{
    convert::TryFrom,
    future::Future,
    io,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use actix_rt::net::ActixStream;
use actix_service::{Service, ServiceFactory};
use actix_utils::future::{ok, Ready};
use futures_core::ready;
use log::trace;
use tokio_rustls::rustls::{client::ServerName, OwnedTrustAnchor, RootCertStore};
use tokio_rustls::{client::TlsStream, rustls::ClientConfig};
use tokio_rustls::{Connect as RustlsConnect, TlsConnector as RustlsTlsConnector};
use webpki_roots::TLS_SERVER_ROOTS;

use crate::connect::{Connection, Host};

pub mod reexports {
    //! Re-exports from `rustls` and `webpki_roots` that are useful for connectors.

    pub use tokio_rustls::{client::TlsStream, rustls::ClientConfig};

    pub use webpki_roots::TLS_SERVER_ROOTS;
}

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

/// Connector service factory using `rustls`.
#[derive(Clone)]
pub struct TlsConnector {
    connector: Arc<ClientConfig>,
}

impl TlsConnector {
    /// Constructs new connector service factory from a `rustls` client configuration.
    pub fn new(connector: Arc<ClientConfig>) -> Self {
        TlsConnector { connector }
    }

    /// Constructs new connector service from a `rustls` client configuration.
    pub fn service(connector: Arc<ClientConfig>) -> TlsConnectorService {
        TlsConnectorService { connector }
    }
}

impl<R, IO> ServiceFactory<Connection<R, IO>> for TlsConnector
where
    R: Host,
    IO: ActixStream + 'static,
{
    type Response = Connection<R, TlsStream<IO>>;
    type Error = io::Error;
    type Config = ();
    type Service = TlsConnectorService;
    type InitError = ();
    type Future = Ready<Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: ()) -> Self::Future {
        ok(TlsConnectorService {
            connector: self.connector.clone(),
        })
    }
}

/// Connector service using `rustls`.
#[derive(Clone)]
pub struct TlsConnectorService {
    connector: Arc<ClientConfig>,
}

impl<R, IO> Service<Connection<R, IO>> for TlsConnectorService
where
    R: Host,
    IO: ActixStream,
{
    type Response = Connection<R, TlsStream<IO>>;
    type Error = io::Error;
    type Future = ConnectFut<R, IO>;

    actix_service::always_ready!();

    fn call(&self, connection: Connection<R, IO>) -> Self::Future {
        trace!("SSL Handshake start for: {:?}", connection.hostname());
        let (stream, connection) = connection.replace_io(());

        match ServerName::try_from(connection.hostname()) {
            Ok(host) => ConnectFut::Future {
                connect: RustlsTlsConnector::from(self.connector.clone()).connect(host, stream),
                connection: Some(connection),
            },
            Err(_) => ConnectFut::InvalidDns,
        }
    }
}

/// Connect future for Rustls service.
#[doc(hidden)]
pub enum ConnectFut<R, IO> {
    /// See issue <https://github.com/briansmith/webpki/issues/54>
    InvalidDns,
    Future {
        connect: RustlsConnect<IO>,
        connection: Option<Connection<R, ()>>,
    },
}

impl<R, IO> Future for ConnectFut<R, IO>
where
    R: Host,
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
                trace!("SSL Handshake success: {:?}", connection.hostname());
                Poll::Ready(Ok(connection.replace_io(stream).1))
            }
        }
    }
}
