use std::{
    future::Future,
    io,
    pin::Pin,
    task::{Context, Poll},
};

use actix_rt::net::ActixStream;
use actix_service::{Service, ServiceFactory};
use futures_core::{future::LocalBoxFuture, ready};
use log::trace;

pub use openssl::ssl::{Error as SslError, HandshakeError, SslConnector, SslMethod};
pub use tokio_openssl::SslStream;

use crate::connect::{Address, Connection};

/// OpenSSL connector factory
pub struct OpensslConnector {
    connector: SslConnector,
}

impl OpensslConnector {
    pub fn new(connector: SslConnector) -> Self {
        OpensslConnector { connector }
    }

    pub fn service(connector: SslConnector) -> OpensslConnectorService {
        OpensslConnectorService { connector }
    }
}

impl Clone for OpensslConnector {
    fn clone(&self) -> Self {
        Self {
            connector: self.connector.clone(),
        }
    }
}

impl<T, U> ServiceFactory<Connection<T, U>> for OpensslConnector
where
    T: Address,
    U: ActixStream + 'static,
{
    type Response = Connection<T, SslStream<U>>;
    type Error = io::Error;
    type Config = ();
    type Service = OpensslConnectorService;
    type InitError = ();
    type Future = LocalBoxFuture<'static, Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: ()) -> Self::Future {
        let connector = self.connector.clone();
        Box::pin(async { Ok(OpensslConnectorService { connector }) })
    }
}

pub struct OpensslConnectorService {
    connector: SslConnector,
}

impl Clone for OpensslConnectorService {
    fn clone(&self) -> Self {
        Self {
            connector: self.connector.clone(),
        }
    }
}

impl<T, U> Service<Connection<T, U>> for OpensslConnectorService
where
    T: Address,
    U: ActixStream,
{
    type Response = Connection<T, SslStream<U>>;
    type Error = io::Error;
    type Future = ConnectAsyncExt<T, U>;

    actix_service::always_ready!();

    fn call(&self, stream: Connection<T, U>) -> Self::Future {
        trace!("SSL Handshake start for: {:?}", stream.host());
        let (io, stream) = stream.replace_io(());
        let host = stream.host();

        let config = self
            .connector
            .configure()
            .expect("SSL connect configuration was invalid.");

        let ssl = config
            .into_ssl(host)
            .expect("SSL connect configuration was invalid.");

        ConnectAsyncExt {
            io: Some(SslStream::new(ssl, io).unwrap()),
            stream: Some(stream),
        }
    }
}

pub struct ConnectAsyncExt<T, U> {
    io: Option<SslStream<U>>,
    stream: Option<Connection<T, ()>>,
}

impl<T: Address, U> Future for ConnectAsyncExt<T, U>
where
    T: Address,
    U: ActixStream,
{
    type Output = Result<Connection<T, SslStream<U>>, io::Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        match ready!(Pin::new(this.io.as_mut().unwrap()).poll_connect(cx)) {
            Ok(_) => {
                let stream = this.stream.take().unwrap();
                trace!("SSL Handshake success: {:?}", stream.host());
                Poll::Ready(Ok(stream.replace_io(this.io.take().unwrap()).1))
            }
            Err(e) => {
                trace!("SSL Handshake error: {:?}", e);
                Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, format!("{}", e))))
            }
        }
    }
}
