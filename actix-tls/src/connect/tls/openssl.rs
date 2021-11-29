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

impl<R, IO> ServiceFactory<Connection<R, IO>> for OpensslConnector
where
    R: Address,
    IO: ActixStream + 'static,
{
    type Response = Connection<R, SslStream<IO>>;
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

impl<R, IO> Service<Connection<R, IO>> for OpensslConnectorService
where
    R: Address,
    IO: ActixStream,
{
    type Response = Connection<R, SslStream<IO>>;
    type Error = io::Error;
    type Future = ConnectAsyncExt<R, IO>;

    actix_service::always_ready!();

    fn call(&self, stream: Connection<R, IO>) -> Self::Future {
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

pub struct ConnectAsyncExt<R, IO> {
    io: Option<SslStream<IO>>,
    stream: Option<Connection<R, ()>>,
}

impl<R: Address, IO> Future for ConnectAsyncExt<R, IO>
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
