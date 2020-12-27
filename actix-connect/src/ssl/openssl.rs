use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::{fmt, io};

pub use open_ssl::ssl::{Error as SslError, SslConnector, SslMethod};
pub use tokio_openssl::SslStream;

use actix_codec::{AsyncRead, AsyncWrite};
use actix_rt::net::TcpStream;
use actix_service::{Service, ServiceFactory};
use futures_util::future::{ready, Ready};
use futures_util::ready;
use trust_dns_resolver::TokioAsyncResolver as AsyncResolver;

use crate::{
    Address, Connect, ConnectError, ConnectService, ConnectServiceFactory, Connection,
};

/// OpenSSL connector factory
pub struct OpensslConnector {
    connector: SslConnector,
}

impl OpensslConnector {
    pub fn new(connector: SslConnector) -> Self {
        OpensslConnector { connector }
    }
}

impl OpensslConnector {
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
    type Future = Ready<Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: ()) -> Self::Future {
        ready(Ok(OpensslConnectorService {
            connector: self.connector.clone(),
        }))
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
    type Future = OpensslConnectorServiceFuture<T, U>;

    actix_service::always_ready!();

    fn call(&mut self, stream: Connection<T, U>) -> Self::Future {
        match self.ssl_stream(stream) {
            Ok(acc) => OpensslConnectorServiceFuture::Accept(Some(acc)),
            Err(e) => OpensslConnectorServiceFuture::Error(Some(e)),
        }
    }
}

impl OpensslConnectorService {
    // construct SslStream with connector.
    // At this point SslStream does not perform any I/O.
    // handshake would happen later in OpensslConnectorServiceFuture
    fn ssl_stream<T, U>(
        &self,
        stream: Connection<T, U>,
    ) -> Result<(SslStream<U>, Connection<T, ()>), SslError>
    where
        T: Address + 'static,
        U: AsyncRead + AsyncWrite + Unpin + fmt::Debug + 'static,
    {
        trace!("SSL Handshake start for: {:?}", stream.host());
        let (stream, connection) = stream.replace(());
        let host = connection.host().to_string();

        let config = self.connector.configure()?;
        let ssl = config.into_ssl(host.as_str())?;
        let stream = tokio_openssl::SslStream::new(ssl, stream)?;
        Ok((stream, connection))
    }
}

#[doc(hidden)]
pub enum OpensslConnectorServiceFuture<T, U>
where
    T: Address + 'static,
    U: AsyncRead + AsyncWrite + Unpin + fmt::Debug + 'static,
{
    Accept(Option<(SslStream<U>, Connection<T, ()>)>),
    Error(Option<SslError>),
}

impl<T, U> Future for OpensslConnectorServiceFuture<T, U>
where
    T: Address,
    U: AsyncRead + AsyncWrite + Unpin + fmt::Debug + 'static,
{
    type Output = Result<Connection<T, SslStream<U>>, io::Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let e = match self.get_mut() {
            Self::Error(e) => e.take().unwrap(),
            Self::Accept(acc) => {
                let (stream, _) = acc.as_mut().unwrap();
                match ready!(Pin::new(stream).poll_connect(cx)) {
                    Ok(()) => {
                        let (stream, connection) = acc.take().unwrap();
                        trace!("SSL Handshake success: {:?}", connection.host());
                        let (_, connection) = connection.replace(stream);
                        return Poll::Ready(Ok(connection));
                    }
                    Err(e) => e,
                }
            }
        };

        trace!("SSL Handshake error: {:?}", e);
        Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, format!("{}", e))))
    }
}

pub struct OpensslConnectServiceFactory<T> {
    tcp: ConnectServiceFactory<T>,
    openssl: OpensslConnector,
}

impl<T> OpensslConnectServiceFactory<T> {
    /// Construct new OpensslConnectService factory
    pub fn new(connector: SslConnector) -> Self {
        OpensslConnectServiceFactory {
            tcp: ConnectServiceFactory::default(),
            openssl: OpensslConnector::new(connector),
        }
    }

    /// Construct new connect service with custom DNS resolver
    pub fn with_resolver(connector: SslConnector, resolver: AsyncResolver) -> Self {
        OpensslConnectServiceFactory {
            tcp: ConnectServiceFactory::with_resolver(resolver),
            openssl: OpensslConnector::new(connector),
        }
    }

    /// Construct OpenSSL connect service
    pub fn service(&self) -> OpensslConnectService<T> {
        OpensslConnectService {
            tcp: self.tcp.service(),
            openssl: OpensslConnectorService {
                connector: self.openssl.connector.clone(),
            },
        }
    }
}

impl<T> Clone for OpensslConnectServiceFactory<T> {
    fn clone(&self) -> Self {
        OpensslConnectServiceFactory {
            tcp: self.tcp.clone(),
            openssl: self.openssl.clone(),
        }
    }
}

impl<T: Address + 'static> ServiceFactory<Connect<T>> for OpensslConnectServiceFactory<T> {
    type Response = SslStream<TcpStream>;
    type Error = ConnectError;
    type Config = ();
    type Service = OpensslConnectService<T>;
    type InitError = ();
    type Future = Ready<Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: ()) -> Self::Future {
        ready(Ok(self.service()))
    }
}

#[derive(Clone)]
pub struct OpensslConnectService<T> {
    tcp: ConnectService<T>,
    openssl: OpensslConnectorService,
}

impl<T: Address + 'static> Service<Connect<T>> for OpensslConnectService<T> {
    type Response = SslStream<TcpStream>;
    type Error = ConnectError;
    type Future = OpensslConnectServiceResponse<T>;

    actix_service::always_ready!();

    fn call(&mut self, req: Connect<T>) -> Self::Future {
        OpensslConnectServiceResponse {
            fut1: Some(self.tcp.call(req)),
            fut2: None,
            openssl: self.openssl.clone(),
        }
    }
}

pub struct OpensslConnectServiceResponse<T: Address + 'static> {
    fut1: Option<<ConnectService<T> as Service<Connect<T>>>::Future>,
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
