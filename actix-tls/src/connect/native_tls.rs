//! Native-TLS based connector service.
//!
//! See [`TlsConnector`] for main connector service factory docs.

use std::io;

use actix_rt::net::ActixStream;
use actix_service::{Service, ServiceFactory};
use actix_utils::future::{ok, Ready};
use futures_core::future::LocalBoxFuture;
use tokio_native_tls::{
    native_tls::TlsConnector as NativeTlsConnector, TlsConnector as AsyncNativeTlsConnector,
    TlsStream as AsyncTlsStream,
};
use tracing::trace;

use crate::connect::{Connection, Host};

pub mod reexports {
    //! Re-exports from `native-tls` and `tokio-native-tls` that are useful for connectors.

    pub use tokio_native_tls::{native_tls::TlsConnector, TlsStream as AsyncTlsStream};
}

/// Connector service and factory using `native-tls`.
#[derive(Clone)]
pub struct TlsConnector {
    connector: AsyncNativeTlsConnector,
}

impl TlsConnector {
    /// Constructs new connector service from a `native-tls` connector.
    ///
    /// This type is it's own service factory, so it can be used in that setting, too.
    pub fn new(connector: NativeTlsConnector) -> Self {
        Self {
            connector: AsyncNativeTlsConnector::from(connector),
        }
    }
}

impl<R: Host, IO> ServiceFactory<Connection<R, IO>> for TlsConnector
where
    IO: ActixStream + 'static,
{
    type Response = Connection<R, AsyncTlsStream<IO>>;
    type Error = io::Error;
    type Config = ();
    type Service = Self;
    type InitError = ();
    type Future = Ready<Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: ()) -> Self::Future {
        ok(self.clone())
    }
}

/// The `native-tls` connector is both it's ServiceFactory and Service impl type.
/// As the factory and service share the same type and state.
impl<R, IO> Service<Connection<R, IO>> for TlsConnector
where
    R: Host,
    IO: ActixStream + 'static,
{
    type Response = Connection<R, AsyncTlsStream<IO>>;
    type Error = io::Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    actix_service::always_ready!();

    fn call(&self, stream: Connection<R, IO>) -> Self::Future {
        let (io, stream) = stream.replace_io(());
        let connector = self.connector.clone();

        Box::pin(async move {
            trace!("TLS handshake start for: {:?}", stream.hostname());
            connector
                .connect(stream.hostname(), io)
                .await
                .map(|res| {
                    trace!("TLS handshake success: {:?}", stream.hostname());
                    stream.replace_io(res).1
                })
                .map_err(|e| {
                    trace!("TLS handshake error: {:?}", e);
                    io::Error::new(io::ErrorKind::Other, format!("{}", e))
                })
        })
    }
}
