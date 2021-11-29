//! OpenSSL based connector service.
//!
//! See [`Connector`] for main connector service factory docs.

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
use log::trace;
use openssl::ssl::{Error as SslError, HandshakeError, SslConnector, SslMethod};
use tokio_openssl::SslStream;

use crate::connect::{Address, Connection};

pub mod reexports {
    //! Re-exports from `openssl` that are useful for connectors.

    pub use openssl::ssl::{Error as SslError, HandshakeError, SslConnector, SslMethod};
}

/// Connector service factory using `openssl`.
pub struct Connector {
    connector: SslConnector,
}

impl Connector {
    /// Constructs new connector service factory from an `openssl` connector.
    pub fn new(connector: SslConnector) -> Self {
        Connector { connector }
    }

    /// Constructs new connector service from an `openssl` connector.
    pub fn service(connector: SslConnector) -> ConnectorService {
        ConnectorService { connector }
    }
}

impl Clone for Connector {
    fn clone(&self) -> Self {
        Self {
            connector: self.connector.clone(),
        }
    }
}

impl<R, IO> ServiceFactory<Connection<R, IO>> for Connector
where
    R: Address,
    IO: ActixStream + 'static,
{
    type Response = Connection<R, SslStream<IO>>;
    type Error = io::Error;
    type Config = ();
    type Service = ConnectorService;
    type InitError = ();
    type Future = Ready<Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: ()) -> Self::Future {
        ok(ConnectorService {
            connector: self.connector.clone(),
        })
    }
}

/// Connector service using `openssl`.
pub struct ConnectorService {
    connector: SslConnector,
}

impl Clone for ConnectorService {
    fn clone(&self) -> Self {
        Self {
            connector: self.connector.clone(),
        }
    }
}

impl<R, IO> Service<Connection<R, IO>> for ConnectorService
where
    R: Address,
    IO: ActixStream,
{
    type Response = Connection<R, SslStream<IO>>;
    type Error = io::Error;
    type Future = ConnectFut<R, IO>;

    actix_service::always_ready!();

    fn call(&self, stream: Connection<R, IO>) -> Self::Future {
        trace!("SSL Handshake start for: {:?}", stream.hostname());
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
            io: Some(SslStream::new(ssl, io).unwrap()),
            stream: Some(stream),
        }
    }
}

/// Connect future for OpenSSL service.
#[doc(hidden)]
pub struct ConnectFut<R, IO> {
    io: Option<SslStream<IO>>,
    stream: Option<Connection<R, ()>>,
}

impl<R: Address, IO> Future for ConnectFut<R, IO>
where
    R: Address,
    IO: ActixStream,
{
    type Output = Result<Connection<R, SslStream<IO>>, io::Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        match ready!(Pin::new(this.io.as_mut().unwrap()).poll_connect(cx)) {
            Ok(_) => {
                let stream = this.stream.take().unwrap();
                trace!("SSL Handshake success: {:?}", stream.hostname());
                Poll::Ready(Ok(stream.replace_io(this.io.take().unwrap()).1))
            }
            Err(e) => {
                trace!("SSL Handshake error: {:?}", e);
                Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, format!("{}", e))))
            }
        }
    }
}
