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
use futures_core::{future::LocalBoxFuture, ready};
use log::{error, trace};
use tokio_util::sync::ReusableBoxFuture;

use super::connect::{Address, Connect, ConnectAddrs, Connection};
use super::error::ConnectError;

/// TCP connector service factory
#[derive(Debug, Copy, Clone)]
pub struct TcpConnectorFactory;

impl TcpConnectorFactory {
    /// Create TCP connector service
    pub fn service(&self) -> TcpConnector {
        TcpConnector
    }
}

impl<T: Address> ServiceFactory<Connect<T>> for TcpConnectorFactory {
    type Response = Connection<T, TcpStream>;
    type Error = ConnectError;
    type Config = ();
    type Service = TcpConnector;
    type InitError = ();
    type Future = LocalBoxFuture<'static, Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: ()) -> Self::Future {
        let service = self.service();
        Box::pin(async move { Ok(service) })
    }
}

/// TCP connector service
#[derive(Debug, Copy, Clone)]
pub struct TcpConnector;

impl<T: Address> Service<Connect<T>> for TcpConnector {
    type Response = Connection<T, TcpStream>;
    type Error = ConnectError;
    type Future = TcpConnectorResponse<T>;

    actix_service::always_ready!();

    fn call(&self, req: Connect<T>) -> Self::Future {
        let port = req.port();
        let Connect {
            req,
            addr,
            local_addr,
            ..
        } = req;

        TcpConnectorResponse::new(req, port, local_addr, addr)
    }
}

/// TCP stream connector response future
pub enum TcpConnectorResponse<T> {
    Response {
        req: Option<T>,
        port: u16,
        local_addr: Option<IpAddr>,
        addrs: Option<VecDeque<SocketAddr>>,
        stream: ReusableBoxFuture<Result<TcpStream, io::Error>>,
    },
    Error(Option<ConnectError>),
}

impl<T: Address> TcpConnectorResponse<T> {
    pub(crate) fn new(
        req: T,
        port: u16,
        local_addr: Option<IpAddr>,
        addr: ConnectAddrs,
    ) -> TcpConnectorResponse<T> {
        if addr.is_none() {
            error!("TCP connector: unresolved connection address");
            return TcpConnectorResponse::Error(Some(ConnectError::Unresolved));
        }

        trace!(
            "TCP connector: connecting to {} on port {}",
            req.hostname(),
            port
        );

        match addr {
            ConnectAddrs::None => unreachable!("none variant already checked"),

            ConnectAddrs::One(addr) => TcpConnectorResponse::Response {
                req: Some(req),
                port,
                local_addr,
                addrs: None,
                stream: ReusableBoxFuture::new(connect(addr, local_addr)),
            },

            // when resolver returns multiple socket addr for request they would be popped from
            // front end of queue and returns with the first successful tcp connection.
            ConnectAddrs::Multi(mut addrs) => {
                let addr = addrs.pop_front().unwrap();

                TcpConnectorResponse::Response {
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

impl<T: Address> Future for TcpConnectorResponse<T> {
    type Output = Result<Connection<T, TcpStream>, ConnectError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.get_mut() {
            TcpConnectorResponse::Error(err) => Poll::Ready(Err(err.take().unwrap())),

            TcpConnectorResponse::Response {
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
                        return Poll::Ready(Ok(Connection::new(sock, req)));
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
    // use local addr if connect asks for it.
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
