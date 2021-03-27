use std::io;

use actix_rt::net::ActixStream;
use actix_service::{Service, ServiceFactory};
use futures_core::future::LocalBoxFuture;
use log::trace;
use tokio_native_tls::{TlsConnector as TokioNativetlsConnector, TlsStream};

pub use tokio_native_tls::native_tls::TlsConnector;

use crate::connect::{Address, Connection};

/// Native-tls connector factory and service
pub struct NativetlsConnector {
    connector: TokioNativetlsConnector,
}

impl NativetlsConnector {
    pub fn new(connector: TlsConnector) -> Self {
        Self {
            connector: TokioNativetlsConnector::from(connector),
        }
    }
}

impl NativetlsConnector {
    pub fn service(connector: TlsConnector) -> Self {
        Self::new(connector)
    }
}

impl Clone for NativetlsConnector {
    fn clone(&self) -> Self {
        Self {
            connector: self.connector.clone(),
        }
    }
}

impl<T: Address, U> ServiceFactory<Connection<T, U>> for NativetlsConnector
where
    U: ActixStream + 'static,
{
    type Response = Connection<T, TlsStream<U>>;
    type Error = io::Error;
    type Config = ();
    type Service = Self;
    type InitError = ();
    type Future = LocalBoxFuture<'static, Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: ()) -> Self::Future {
        let connector = self.clone();
        Box::pin(async { Ok(connector) })
    }
}

// NativetlsConnector is both it's ServiceFactory and Service impl type.
// As the factory and service share the same type and state.
impl<T, U> Service<Connection<T, U>> for NativetlsConnector
where
    T: Address,
    U: ActixStream + 'static,
{
    type Response = Connection<T, TlsStream<U>>;
    type Error = io::Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    actix_service::always_ready!();

    fn call(&self, stream: Connection<T, U>) -> Self::Future {
        let (io, stream) = stream.replace_io(());
        let connector = self.connector.clone();
        Box::pin(async move {
            trace!("SSL Handshake start for: {:?}", stream.host());
            connector
                .connect(stream.host(), io)
                .await
                .map(|res| {
                    trace!("SSL Handshake success: {:?}", stream.host());
                    stream.replace_io(res).1
                })
                .map_err(|e| {
                    trace!("SSL Handshake error: {:?}", e);
                    io::Error::new(io::ErrorKind::Other, format!("{}", e))
                })
        })
    }
}
