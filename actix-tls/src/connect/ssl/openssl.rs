use std::{
    fmt,
    future::Future,
    io,
    pin::Pin,
    task::{Context, Poll},
};

use actix_codec::{AsyncRead, AsyncWrite};
use actix_rt::net::TcpStream;
use actix_service::{Service, ServiceFactory};
use futures_core::{future::LocalBoxFuture, ready};
use log::trace;

pub use openssl::ssl::{Error as SslError, HandshakeError, SslConnector, SslMethod};
pub use tokio_openssl::SslStream;

use crate::connect::resolve::Resolve;
use crate::connect::{
    Address, Connect, ConnectError, ConnectService, ConnectServiceFactory, Connection, Resolver,
};

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
    T: Address + 'static,
    U: AsyncRead + AsyncWrite + Unpin + fmt::Debug + 'static,
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
    T: Address + 'static,
    U: AsyncRead + AsyncWrite + Unpin + fmt::Debug + 'static,
{
    type Response = Connection<T, SslStream<U>>;
    type Error = io::Error;
    type Future = ConnectAsyncExt<T, U>;

    actix_service::always_ready!();

    fn call(&self, stream: Connection<T, U>) -> Self::Future {
        trace!("SSL Handshake start for: {:?}", stream.host());
        let (io, stream) = stream.replace(());
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
    U: AsyncRead + AsyncWrite + Unpin + fmt::Debug + 'static,
{
    type Output = Result<Connection<T, SslStream<U>>, io::Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        match ready!(Pin::new(this.io.as_mut().unwrap()).poll_connect(cx)) {
            Ok(_) => {
                let stream = this.stream.take().unwrap();
                trace!("SSL Handshake success: {:?}", stream.host());
                Poll::Ready(Ok(stream.replace(this.io.take().unwrap()).1))
            }
            Err(e) => {
                trace!("SSL Handshake error: {:?}", e);
                Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, format!("{}", e))))
            }
        }
    }
}

pub struct OpensslConnectServiceFactory {
    tcp: ConnectServiceFactory,
    openssl: OpensslConnector,
}

impl OpensslConnectServiceFactory {
    /// Construct new OpensslConnectService factory
    pub fn new(connector: SslConnector) -> Self {
        OpensslConnectServiceFactory {
            tcp: ConnectServiceFactory::new(Resolver::Default),
            openssl: OpensslConnector::new(connector),
        }
    }

    /// Construct new connect service with custom DNS resolver
    pub fn with_resolver(connector: SslConnector, resolver: impl Resolve + 'static) -> Self {
        OpensslConnectServiceFactory {
            tcp: ConnectServiceFactory::new(Resolver::new_custom(resolver)),
            openssl: OpensslConnector::new(connector),
        }
    }

    /// Construct OpenSSL connect service
    pub fn service(&self) -> OpensslConnectService {
        OpensslConnectService {
            tcp: self.tcp.service(),
            openssl: OpensslConnectorService {
                connector: self.openssl.connector.clone(),
            },
        }
    }
}

impl Clone for OpensslConnectServiceFactory {
    fn clone(&self) -> Self {
        OpensslConnectServiceFactory {
            tcp: self.tcp.clone(),
            openssl: self.openssl.clone(),
        }
    }
}

impl<T: Address + 'static> ServiceFactory<Connect<T>> for OpensslConnectServiceFactory {
    type Response = SslStream<TcpStream>;
    type Error = ConnectError;
    type Config = ();
    type Service = OpensslConnectService;
    type InitError = ();
    type Future = LocalBoxFuture<'static, Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: ()) -> Self::Future {
        let service = self.service();
        Box::pin(async { Ok(service) })
    }
}

#[derive(Clone)]
pub struct OpensslConnectService {
    tcp: ConnectService,
    openssl: OpensslConnectorService,
}

impl<T: Address + 'static> Service<Connect<T>> for OpensslConnectService {
    type Response = SslStream<TcpStream>;
    type Error = ConnectError;
    type Future = OpensslConnectServiceResponse<T>;

    actix_service::always_ready!();

    fn call(&self, req: Connect<T>) -> Self::Future {
        OpensslConnectServiceResponse {
            fut1: Some(self.tcp.call(req)),
            fut2: None,
            openssl: self.openssl.clone(),
        }
    }
}

pub struct OpensslConnectServiceResponse<T: Address + 'static> {
    fut1: Option<<ConnectService as Service<Connect<T>>>::Future>,
    fut2: Option<<OpensslConnectorService as Service<Connection<T, TcpStream>>>::Future>,
    openssl: OpensslConnectorService,
}

impl<T: Address> Future for OpensslConnectServiceResponse<T> {
    type Output = Result<SslStream<TcpStream>, ConnectError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if let Some(ref mut fut) = self.fut1 {
            match ready!(Pin::new(fut).poll(cx)) {
                Ok(res) => {
                    let _ = self.fut1.take();
                    self.fut2 = Some(self.openssl.call(res));
                }
                Err(e) => return Poll::Ready(Err(e)),
            }
        }

        if let Some(ref mut fut) = self.fut2 {
            match ready!(Pin::new(fut).poll(cx)) {
                Ok(connect) => Poll::Ready(Ok(connect.into_parts().0)),
                Err(e) => Poll::Ready(Err(ConnectError::Io(io::Error::new(
                    io::ErrorKind::Other,
                    e,
                )))),
            }
        } else {
            Poll::Pending
        }
    }
}
