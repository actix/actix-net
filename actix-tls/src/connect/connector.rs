use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use actix_rt::net::TcpStream;
use actix_service::{Service, ServiceFactory};
use futures_core::{future::LocalBoxFuture, ready};

use super::{
    error::ConnectError,
    resolver::{Resolver, ResolverService},
    tcp::{TcpConnector, TcpConnectorService},
    Address, Connection, ConnectionInfo,
};

/// Combined resolver and TCP connector service factory.
///
/// Used to create [`ConnectService`]s which receive connection information, resolve DNS if
/// required, and return a TCP stream.
pub struct Connector {
    tcp: TcpConnector,
    resolver: Resolver,
}

impl Connector {
    /// Constructs new connector factory.
    pub fn new(resolver: Resolver) -> Self {
        Connector {
            tcp: TcpConnector,
            resolver,
        }
    }

    /// Build connector service.
    pub fn service(&self) -> ConnectorService {
        ConnectorService {
            tcp: self.tcp.service(),
            resolver: self.resolver.service(),
        }
    }
}

impl Clone for Connector {
    fn clone(&self) -> Self {
        Connector {
            tcp: self.tcp,
            resolver: self.resolver.clone(),
        }
    }
}

impl Default for Connector {
    fn default() -> Self {
        Self {
            tcp: TcpConnector,
            resolver: Resolver::default(),
        }
    }
}

impl<R: Address> ServiceFactory<ConnectionInfo<R>> for Connector {
    type Response = Connection<R, TcpStream>;
    type Error = ConnectError;
    type Config = ();
    type Service = ConnectorService;
    type InitError = ();
    type Future = LocalBoxFuture<'static, Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: ()) -> Self::Future {
        let service = self.service();
        Box::pin(async { Ok(service) })
    }
}

/// Combined resolver and TCP connector service.
///
/// Service implementation receives connection information, resolves DNS if required, and returns
/// a TCP stream.
#[derive(Clone)]
pub struct ConnectorService {
    tcp: TcpConnectorService,
    resolver: ResolverService,
}

impl<R: Address> Service<ConnectionInfo<R>> for ConnectorService {
    type Response = Connection<R, TcpStream>;
    type Error = ConnectError;
    type Future = ConnectServiceResponse<R>;

    actix_service::always_ready!();

    fn call(&self, req: ConnectionInfo<R>) -> Self::Future {
        ConnectServiceResponse {
            fut: ConnectFuture::Resolve(self.resolver.call(req)),
            tcp: self.tcp,
        }
    }
}

// helper enum to generic over futures of resolve and connect phase.
pub(crate) enum ConnectFuture<R: Address> {
    Resolve(<ResolverService as Service<ConnectionInfo<R>>>::Future),
    Connect(<TcpConnectorService as Service<ConnectionInfo<R>>>::Future),
}

/// Helper enum to contain the future output of `ConnectFuture`.
pub(crate) enum ConnectOutput<R: Address> {
    Resolved(ConnectionInfo<R>),
    Connected(Connection<R, TcpStream>),
}

impl<R: Address> ConnectFuture<R> {
    fn poll_connect(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Result<ConnectOutput<R>, ConnectError>> {
        match self {
            ConnectFuture::Resolve(ref mut fut) => {
                Pin::new(fut).poll(cx).map_ok(ConnectOutput::Resolved)
            }
            ConnectFuture::Connect(ref mut fut) => {
                Pin::new(fut).poll(cx).map_ok(ConnectOutput::Connected)
            }
        }
    }
}

pub struct ConnectServiceResponse<R: Address> {
    fut: ConnectFuture<R>,
    tcp: TcpConnectorService,
}

impl<R: Address> Future for ConnectServiceResponse<R> {
    type Output = Result<Connection<R, TcpStream>, ConnectError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        loop {
            match ready!(self.fut.poll_connect(cx))? {
                ConnectOutput::Resolved(res) => {
                    self.fut = ConnectFuture::Connect(self.tcp.call(res));
                }
                ConnectOutput::Connected(res) => return Poll::Ready(Ok(res)),
            }
        }
    }
}
