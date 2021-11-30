use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use actix_rt::net::TcpStream;
use actix_service::{Service, ServiceFactory};
use actix_utils::future::{ok, Ready};
use futures_core::ready;

use super::{
    error::ConnectError,
    resolver::{Resolver, ResolverService},
    tcp::{TcpConnector, TcpConnectorService},
    ConnectInfo, Connection, Host,
};

/// Combined resolver and TCP connector service factory.
///
/// Used to create [`ConnectorService`]s which receive connection information, resolve DNS if
/// required, and return a TCP stream.
#[derive(Clone, Default)]
pub struct Connector {
    resolver: Resolver,
}

impl Connector {
    /// Constructs new connector factory with the given resolver.
    pub fn new(resolver: Resolver) -> Self {
        Connector { resolver }
    }

    /// Build connector service.
    pub fn service(&self) -> ConnectorService {
        ConnectorService {
            tcp: TcpConnector::default().service(),
            resolver: self.resolver.service(),
        }
    }
}

impl<R: Host> ServiceFactory<ConnectInfo<R>> for Connector {
    type Response = Connection<R, TcpStream>;
    type Error = ConnectError;
    type Config = ();
    type Service = ConnectorService;
    type InitError = ();
    type Future = Ready<Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: ()) -> Self::Future {
        ok(self.service())
    }
}

/// Combined resolver and TCP connector service.
///
/// Service implementation receives connection information, resolves DNS if required, and returns
/// a TCP stream.
#[derive(Clone, Default)]
pub struct ConnectorService {
    tcp: TcpConnectorService,
    resolver: ResolverService,
}

impl<R: Host> Service<ConnectInfo<R>> for ConnectorService {
    type Response = Connection<R, TcpStream>;
    type Error = ConnectError;
    type Future = ConnectServiceResponse<R>;

    actix_service::always_ready!();

    fn call(&self, req: ConnectInfo<R>) -> Self::Future {
        ConnectServiceResponse {
            fut: ConnectFut::Resolve(self.resolver.call(req)),
            tcp: self.tcp,
        }
    }
}

/// Chains futures of resolve and connect steps.
pub(crate) enum ConnectFut<R: Host> {
    Resolve(<ResolverService as Service<ConnectInfo<R>>>::Future),
    Connect(<TcpConnectorService as Service<ConnectInfo<R>>>::Future),
}

/// Container for the intermediate states of [`ConnectFut`].
pub(crate) enum ConnectFutState<R: Host> {
    Resolved(ConnectInfo<R>),
    Connected(Connection<R, TcpStream>),
}

impl<R: Host> ConnectFut<R> {
    fn poll_connect(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Result<ConnectFutState<R>, ConnectError>> {
        match self {
            ConnectFut::Resolve(ref mut fut) => {
                Pin::new(fut).poll(cx).map_ok(ConnectFutState::Resolved)
            }

            ConnectFut::Connect(ref mut fut) => {
                Pin::new(fut).poll(cx).map_ok(ConnectFutState::Connected)
            }
        }
    }
}

pub struct ConnectServiceResponse<R: Host> {
    fut: ConnectFut<R>,
    tcp: TcpConnectorService,
}

impl<R: Host> Future for ConnectServiceResponse<R> {
    type Output = Result<Connection<R, TcpStream>, ConnectError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        loop {
            match ready!(self.fut.poll_connect(cx))? {
                ConnectFutState::Resolved(res) => {
                    self.fut = ConnectFut::Connect(self.tcp.call(res));
                }
                ConnectFutState::Connected(res) => return Poll::Ready(Ok(res)),
            }
        }
    }
}
