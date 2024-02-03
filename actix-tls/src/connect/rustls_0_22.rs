//! Rustls based connector service.
//!
//! See [`TlsConnector`] for main connector service factory docs.

use std::{
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
use rustls_pki_types_1::ServerName;
use tokio_rustls::{
    client::TlsStream as AsyncTlsStream, rustls::ClientConfig, Connect as RustlsConnect,
    TlsConnector as RustlsTlsConnector,
};
use tokio_rustls_025 as tokio_rustls;

use crate::connect::{Connection, Host};

pub mod reexports {
    //! Re-exports from the `rustls` v0.22 ecosystem that are useful for connectors.

    pub use tokio_rustls_025::{client::TlsStream as AsyncTlsStream, rustls::ClientConfig};
    #[cfg(feature = "rustls-0_22-webpki-roots")]
    pub use webpki_roots_026::TLS_SERVER_ROOTS;
}

/// Returns root certificates via `rustls-native-certs` crate as a rustls certificate store.
///
/// See [`rustls_native_certs::load_native_certs()`] for more info on behavior and errors.
#[cfg(feature = "rustls-0_22-native-roots")]
pub fn native_roots_cert_store() -> io::Result<tokio_rustls::rustls::RootCertStore> {
    let mut root_certs = tokio_rustls::rustls::RootCertStore::empty();

    for cert in rustls_native_certs_07::load_native_certs()? {
        root_certs.add(cert).unwrap();
    }

    Ok(root_certs)
}

/// Returns standard root certificates from `webpki-roots` crate as a rustls certificate store.
#[cfg(feature = "rustls-0_22-webpki-roots")]
pub fn webpki_roots_cert_store() -> tokio_rustls::rustls::RootCertStore {
    let mut root_certs = tokio_rustls::rustls::RootCertStore::empty();
    root_certs.extend(webpki_roots_026::TLS_SERVER_ROOTS.to_owned());
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
    type Response = Connection<R, AsyncTlsStream<IO>>;
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
    type Response = Connection<R, AsyncTlsStream<IO>>;
    type Error = io::Error;
    type Future = ConnectFut<R, IO>;

    actix_service::always_ready!();

    fn call(&self, connection: Connection<R, IO>) -> Self::Future {
        tracing::trace!("TLS handshake start for: {:?}", connection.hostname());
        let (stream, conn) = connection.replace_io(());

        match ServerName::try_from(conn.hostname()) {
            Ok(host) => ConnectFut::Future {
                connect: RustlsTlsConnector::from(Arc::clone(&self.connector))
                    .connect(host.to_owned(), stream),
                connection: Some(conn),
            },
            Err(_) => ConnectFut::InvalidServerName,
        }
    }
}

/// Connect future for Rustls service.
#[doc(hidden)]
#[allow(clippy::large_enum_variant)]
pub enum ConnectFut<R, IO> {
    InvalidServerName,
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
    type Output = io::Result<Connection<R, AsyncTlsStream<IO>>>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.get_mut() {
            Self::InvalidServerName => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "connection parameters specified invalid server name",
            ))),

            Self::Future {
                connect,
                connection,
            } => {
                let stream = ready!(Pin::new(connect).poll(cx))?;
                let connection = connection.take().unwrap();
                tracing::trace!("TLS handshake success: {:?}", connection.hostname());
                Poll::Ready(Ok(connection.replace_io(stream).1))
            }
        }
    }
}
