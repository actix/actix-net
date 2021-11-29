use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use actix_rt::net::TcpStream;
use actix_service::{Service, ServiceFactory};
use futures_core::{future::LocalBoxFuture, ready};

use super::connect::{Address, Connect, Connection};
use super::connector::{TcpConnector, TcpConnectorFactory};
use super::error::ConnectError;
use super::resolve::{Resolver, ResolverFactory};

pub struct ConnectServiceFactory {
    tcp: TcpConnectorFactory,
    resolver: ResolverFactory,
}

impl ConnectServiceFactory {
    /// Constructs new ConnectService factory.
    pub fn new(resolver: Resolver) -> Self {
        ConnectServiceFactory {
            tcp: TcpConnectorFactory,
            resolver: ResolverFactory::new(resolver),
        }
    }

    /// Constructs new service.
    pub fn service(&self) -> ConnectService {
        ConnectService {
            tcp: self.tcp.service(),
            resolver: self.resolver.service(),
        }
    }
}

impl Clone for ConnectServiceFactory {
    fn clone(&self) -> Self {
        ConnectServiceFactory {
            tcp: self.tcp,
            resolver: self.resolver.clone(),
        }
    }
}

impl<R: Address> ServiceFactory<Connect<R>> for ConnectServiceFactory {
    type Response = Connection<R, TcpStream>;
    type Error = ConnectError;
    type Config = ();
    type Service = ConnectService;
    type InitError = ();
    type Future = LocalBoxFuture<'static, Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: ()) -> Self::Future {
        let service = self.service();
        Box::pin(async { Ok(service) })
    }
}

#[derive(Clone)]
pub struct ConnectService {
    tcp: TcpConnector,
    resolver: Resolver,
}

impl<R: Address> Service<Connect<R>> for ConnectService {
    type Response = Connection<R, TcpStream>;
    type Error = ConnectError;
    type Future = ConnectServiceResponse<R>;

    actix_service::always_ready!();

    fn call(&self, req: Connect<R>) -> Self::Future {
        ConnectServiceResponse {
            fut: ConnectFuture::Resolve(self.resolver.call(req)),
            tcp: self.tcp,
        }
    }
}

// helper enum to generic over futures of resolve and connect phase.
pub(crate) enum ConnectFuture<R: Address> {
    Resolve(<Resolver as Service<Connect<R>>>::Future),
    Connect(<TcpConnector as Service<Connect<R>>>::Future),
}

/// Helper enum to contain the future output of `ConnectFuture`.
pub(crate) enum ConnectOutput<R: Address> {
    Resolved(Connect<R>),
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
    tcp: TcpConnector,
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
