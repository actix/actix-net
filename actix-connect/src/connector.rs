use std::collections::VecDeque;
use std::future::Future;
use std::io;
use std::marker::PhantomData;
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{Context, Poll};

use actix_rt::net::TcpStream;
use actix_service::{Service, ServiceFactory};
use futures_util::future::{ready, Ready};

use super::connect::{Address, Connect, Connection};
use super::error::ConnectError;

/// TCP connector service factory
#[derive(Debug)]
pub struct TcpConnectorFactory<T>(PhantomData<T>);

impl<T> TcpConnectorFactory<T> {
    pub fn new() -> Self {
        TcpConnectorFactory(PhantomData)
    }

    /// Create TCP connector service
    pub fn service(&self) -> TcpConnector<T> {
        TcpConnector(PhantomData)
    }
}

impl<T> Default for TcpConnectorFactory<T> {
    fn default() -> Self {
        TcpConnectorFactory(PhantomData)
    }
}

impl<T> Clone for TcpConnectorFactory<T> {
    fn clone(&self) -> Self {
        TcpConnectorFactory(PhantomData)
    }
}

impl<T: Address> ServiceFactory<Connect<T>> for TcpConnectorFactory<T> {
    type Response = Connection<T, TcpStream>;
    type Error = ConnectError;
    type Config = ();
    type Service = TcpConnector<T>;
    type InitError = ();
    type Future = Ready<Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: ()) -> Self::Future {
        ready(Ok(self.service()))
    }
}

/// TCP connector service
#[derive(Default, Debug)]
pub struct TcpConnector<T>(PhantomData<T>);

impl<T> TcpConnector<T> {
    pub fn new() -> Self {
        TcpConnector(PhantomData)
    }
}

impl<T> Clone for TcpConnector<T> {
    fn clone(&self) -> Self {
        TcpConnector(PhantomData)
    }
}

impl<T: Address> Service<Connect<T>> for TcpConnector<T> {
    type Response = Connection<T, TcpStream>;
    type Error = ConnectError;
    type Future = TcpConnectorResponse<T>;

    actix_service::always_ready!();

    fn call(&mut self, req: Connect<T>) -> Self::Future {
        let port = req.port();
        let Connect { req, addr, .. } = req;

        if let Some(addr) = addr {
            TcpConnectorResponse::new(req, port, addr)
        } else {
            error!("TCP connector: got unresolved address");
            TcpConnectorResponse::Error(Some(ConnectError::Unresolved))
        }
    }
}

type LocalBoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

#[doc(hidden)]
/// TCP stream connector response future
pub enum TcpConnectorResponse<T> {
    Response {
        req: Option<T>,
        port: u16,
        addrs: Option<VecDeque<SocketAddr>>,
        stream: Option<LocalBoxFuture<'static, Result<TcpStream, io::Error>>>,
    },
    Error(Option<ConnectError>),
}

impl<T: Address> TcpConnectorResponse<T> {
    pub fn new(
        req: T,
        port: u16,
        addr: either::Either<SocketAddr, VecDeque<SocketAddr>>,
    ) -> TcpConnectorResponse<T> {
        trace!(
            "TCP connector - connecting to {:?} port:{}",
            req.host(),
            port
        );

        match addr {
            either::Either::Left(addr) => TcpConnectorResponse::Response {
                req: Some(req),
                port,
                addrs: None,
                stream: Some(Box::pin(TcpStream::connect(addr))),
            },
            either::Either::Right(addrs) => TcpConnectorResponse::Response {
                req: Some(req),
                port,
                addrs: Some(addrs),
                stream: None,
            },
        }
    }
}

impl<T: Address> Future for TcpConnectorResponse<T> {
    type Output = Result<Connection<T, TcpStream>, ConnectError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        match this {
            TcpConnectorResponse::Error(e) => Poll::Ready(Err(e.take().unwrap())),
            // connect
            TcpConnectorResponse::Response {
                req,
                port,
                addrs,
                stream,
            } => loop {
                if let Some(new) = stream.as_mut() {
                    match new.as_mut().poll(cx) {
                        Poll::Ready(Ok(sock)) => {
                            let req = req.take().unwrap();
                            trace!(
                                    "TCP connector - successfully connected to connecting to {:?} - {:?}",
                                    req.host(), sock.peer_addr()
                                );
                            return Poll::Ready(Ok(Connection::new(sock, req)));
                        }
                        Poll::Pending => return Poll::Pending,
                        Poll::Ready(Err(err)) => {
                            trace!(
                                    "TCP connector - failed to connect to connecting to {:?} port: {}",
                                    req.as_ref().unwrap().host(),
                                    port,
                                );
                            if addrs.is_none() || addrs.as_ref().unwrap().is_empty() {
                                return Poll::Ready(Err(err.into()));
                            }
                        }
                    }
                }

                // try to connect
                let addr = addrs.as_mut().unwrap().pop_front().unwrap();
                *stream = Some(Box::pin(TcpStream::connect(addr)));
            },
        }
    }
}
