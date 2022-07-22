//! TCP connector service.
//!
//! See [`TcpConnector`] for main connector service factory docs.

use std::{
    collections::VecDeque,
    future::Future,
    io,
    net::{IpAddr, SocketAddr, SocketAddrV4, SocketAddrV6},
    pin::Pin,
    task::{Context, Poll},
};

use actix_rt::net::{TcpSocket, TcpStream};
use actix_service::{Service, ServiceFactory};
use actix_utils::future::{ok, Ready};
use futures_core::ready;
use tokio_util::sync::ReusableBoxFuture;
use tracing::{error, trace};

use super::{connect_addrs::ConnectAddrs, error::ConnectError, ConnectInfo, Connection, Host};

/// TCP connector service factory.
#[derive(Debug, Clone, Copy, Default)]
#[non_exhaustive]
pub struct TcpConnector;

impl TcpConnector {
    /// Returns a new TCP connector service.
    pub fn service(&self) -> TcpConnectorService {
        TcpConnectorService::default()
    }
}

impl<R: Host> ServiceFactory<ConnectInfo<R>> for TcpConnector {
    type Response = Connection<R, TcpStream>;
    type Error = ConnectError;
    type Config = ();
    type Service = TcpConnectorService;
    type InitError = ();
    type Future = Ready<Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: ()) -> Self::Future {
        ok(self.service())
    }
}

/// TCP connector service.
#[derive(Debug, Copy, Clone, Default)]
#[non_exhaustive]
pub struct TcpConnectorService;

impl<R: Host> Service<ConnectInfo<R>> for TcpConnectorService {
    type Response = Connection<R, TcpStream>;
    type Error = ConnectError;
    type Future = TcpConnectorFut<R>;

    actix_service::always_ready!();

    fn call(&self, req: ConnectInfo<R>) -> Self::Future {
        let port = req.port();

        let ConnectInfo {
            request: req,
            addr,
            local_addr,
            ..
        } = req;

        TcpConnectorFut::new(req, port, local_addr, addr)
    }
}

/// Connect future for TCP service.
#[doc(hidden)]
pub enum TcpConnectorFut<R> {
    Response {
        req: Option<R>,
        port: u16,
        local_addr: Option<IpAddr>,
        addrs: Option<VecDeque<SocketAddr>>,
        stream: ReusableBoxFuture<'static, Result<TcpStream, io::Error>>,
    },

    Error(Option<ConnectError>),
}

impl<R: Host> TcpConnectorFut<R> {
    pub(crate) fn new(
        req: R,
        port: u16,
        local_addr: Option<IpAddr>,
        addr: ConnectAddrs,
    ) -> TcpConnectorFut<R> {
        if addr.is_unresolved() {
            error!("TCP connector: unresolved connection address");
            return TcpConnectorFut::Error(Some(ConnectError::Unresolved));
        }

        trace!(
            "TCP connector: connecting to {} on port {}",
            req.hostname(),
            port
        );

        match addr {
            ConnectAddrs::None => unreachable!("none variant already checked"),

            ConnectAddrs::One(addr) => TcpConnectorFut::Response {
                req: Some(req),
                port,
                local_addr,
                addrs: None,
                stream: ReusableBoxFuture::new(connect(addr, local_addr)),
            },

            // When resolver returns multiple socket addr for request they would be popped from
            // front end of queue and returns with the first successful TCP connection.
            ConnectAddrs::Multi(mut addrs) => {
                let addr = addrs.pop_front().unwrap();

                TcpConnectorFut::Response {
                    req: Some(req),
                    port,
                    local_addr,
                    addrs: Some(addrs),
                    stream: ReusableBoxFuture::new(connect(addr, local_addr)),
                }
            }
        }
    }
}

impl<R: Host> Future for TcpConnectorFut<R> {
    type Output = Result<Connection<R, TcpStream>, ConnectError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.get_mut() {
            TcpConnectorFut::Error(err) => Poll::Ready(Err(err.take().unwrap())),

            TcpConnectorFut::Response {
                req,
                port,
                local_addr,
                addrs,
                stream,
            } => loop {
                match ready!(stream.poll(cx)) {
                    Ok(sock) => {
                        let req = req.take().unwrap();

                        trace!(
                            "TCP connector: successfully connected to {:?} - {:?}",
                            req.hostname(),
                            sock.peer_addr()
                        );

                        return Poll::Ready(Ok(Connection::new(req, sock)));
                    }

                    Err(err) => {
                        trace!(
                            "TCP connector: failed to connect to {:?} port: {}",
                            req.as_ref().unwrap().hostname(),
                            port,
                        );

                        if let Some(addr) = addrs.as_mut().and_then(|addrs| addrs.pop_front()) {
                            stream.set(connect(addr, *local_addr));
                        } else {
                            return Poll::Ready(Err(ConnectError::Io(err)));
                        }
                    }
                }
            },
        }
    }
}

async fn connect(addr: SocketAddr, local_addr: Option<IpAddr>) -> io::Result<TcpStream> {
    // use local addr if connect asks for it
    match local_addr {
        Some(ip_addr) => {
            let socket = match ip_addr {
                IpAddr::V4(ip_addr) => {
                    let socket = TcpSocket::new_v4()?;
                    let addr = SocketAddr::V4(SocketAddrV4::new(ip_addr, 0));
                    socket.bind(addr)?;
                    socket
                }
                IpAddr::V6(ip_addr) => {
                    let socket = TcpSocket::new_v6()?;
                    let addr = SocketAddr::V6(SocketAddrV6::new(ip_addr, 0, 0, 0));
                    socket.bind(addr)?;
                    socket
                }
            };

            socket.connect(addr).await
        }

        None => TcpStream::connect(addr).await,
    }
}
