//! OpenSSL based connector service.
//!
//! See [`TlsConnector`] for main connector service factory docs.

use std::{
    future::Future,
    io,
    pin::Pin,
    task::{Context, Poll},
};

use actix_rt::net::ActixStream;
use actix_service::{Service, ServiceFactory};
use actix_utils::future::{ok, Ready};
use futures_core::ready;
use openssl::ssl::SslConnector;
use tokio_openssl::SslStream as AsyncSslStream;
use tracing::trace;

use crate::connect::{Connection, Host};

pub mod reexports {
    //! Re-exports from `openssl` and `tokio-openssl` that are useful for connectors.

    pub use openssl::ssl::{Error, HandshakeError, SslConnector, SslConnectorBuilder, SslMethod};
    pub use tokio_openssl::SslStream as AsyncSslStream;
}

/// Connector service factory using `openssl`.
pub struct TlsConnector {
    connector: SslConnector,
}

impl TlsConnector {
    /// Constructs new connector service factory from an `openssl` connector.
    pub fn new(connector: SslConnector) -> Self {
        TlsConnector { connector }
    }

    /// Constructs new connector service from an `openssl` connector.
    pub fn service(connector: SslConnector) -> TlsConnectorService {
        TlsConnectorService { connector }
    }
}

impl Clone for TlsConnector {
    fn clone(&self) -> Self {
        Self {
            connector: self.connector.clone(),
        }
    }
}

impl<R, IO> ServiceFactory<Connection<R, IO>> for TlsConnector
where
    R: Host,
    IO: ActixStream + 'static,
{
    type Response = Connection<R, AsyncSslStream<IO>>;
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

/// Connector service using `openssl`.
pub struct TlsConnectorService {
    connector: SslConnector,
}

impl Clone for TlsConnectorService {
    fn clone(&self) -> Self {
        Self {
            connector: self.connector.clone(),
        }
    }
}

impl<R, IO> Service<Connection<R, IO>> for TlsConnectorService
where
    R: Host,
    IO: ActixStream,
{
    type Response = Connection<R, AsyncSslStream<IO>>;
    type Error = io::Error;
    type Future = ConnectFut<R, IO>;

    actix_service::always_ready!();

    fn call(&self, stream: Connection<R, IO>) -> Self::Future {
        trace!("TLS handshake start for: {:?}", stream.hostname());

        let (io, stream) = stream.replace_io(());
        let host = stream.hostname();

        let config = self
            .connector
            .configure()
            .expect("SSL connect configuration was invalid.");

        let ssl = config
            .into_ssl(host)
            .expect("SSL connect configuration was invalid.");

        ConnectFut {
            io: Some(AsyncSslStream::new(ssl, io).unwrap()),
            stream: Some(stream),
        }
    }
}

/// Connect future for OpenSSL service.
#[doc(hidden)]
pub struct ConnectFut<R, IO> {
    io: Option<AsyncSslStream<IO>>,
    stream: Option<Connection<R, ()>>,
}

impl<R: Host, IO> Future for ConnectFut<R, IO>
where
    R: Host,
    IO: ActixStream,
{
    type Output = Result<Connection<R, AsyncSslStream<IO>>, io::Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        match ready!(Pin::new(this.io.as_mut().unwrap()).poll_connect(cx)) {
            Ok(_) => {
                let stream = this.stream.take().unwrap();
                trace!("TLS handshake success: {:?}", stream.hostname());
                Poll::Ready(Ok(stream.replace_io(this.io.take().unwrap()).1))
            }
            Err(err) => {
                trace!("TLS handshake error: {:?}", err);
                Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::Other,
                    format!("{}", err),
                )))
            }
        }
    }
}
