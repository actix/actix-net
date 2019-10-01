use std::marker::PhantomData;

use actix_service::{NewService, Service};
use futures::{future::ok, future::Ready, Future, FutureExt, Poll};
use openssl::ssl::SslAcceptor;
use tokio_io::{AsyncRead, AsyncWrite};
use tokio_openssl::{HandshakeError, SslStream};

use crate::counter::{Counter, CounterGuard};
use crate::ssl::MAX_CONN_COUNTER;
use crate::{Io, Protocol, ServerConfig};
use futures::future::LocalBoxFuture;
use std::io;
use std::pin::Pin;
use std::task::Context;

/// Support `SSL` connections via openssl package
///
/// `ssl` feature enables `OpensslAcceptor` type
pub struct OpensslAcceptor<T: AsyncRead + AsyncWrite, P = ()> {
    acceptor: SslAcceptor,
    io: PhantomData<(T, P)>,
}

impl<T: AsyncRead + AsyncWrite, P> OpensslAcceptor<T, P> {
    /// Create default `OpensslAcceptor`
    pub fn new(acceptor: SslAcceptor) -> Self {
        OpensslAcceptor {
            acceptor,
            io: PhantomData,
        }
    }
}

impl<T: AsyncRead + AsyncWrite, P> Clone for OpensslAcceptor<T, P> {
    fn clone(&self) -> Self {
        Self {
            acceptor: self.acceptor.clone(),
            io: PhantomData,
        }
    }
}

impl<T: AsyncRead + AsyncWrite + Unpin + 'static, P> NewService for OpensslAcceptor<T, P> {
    type Request = Io<T, P>;
    type Response = Io<SslStream<T>, P>;
    type Error = HandshakeError<T>;
    type Config = ServerConfig;
    type Service = OpensslAcceptorService<T, P>;
    type InitError = ();
    type Future = Ready<Result<Self::Service, Self::InitError>>;

    fn new_service(&self, cfg: &ServerConfig) -> Self::Future {
        cfg.set_secure();

        MAX_CONN_COUNTER.with(|conns| {
            ok(OpensslAcceptorService {
                acceptor: self.acceptor.clone(),
                conns: conns.clone(),
                io: PhantomData,
            })
        })
    }
}

pub struct OpensslAcceptorService<T, P> {
    acceptor: SslAcceptor,
    conns: Counter,
    io: PhantomData<(T, P)>,
}

impl<T: AsyncRead + AsyncWrite + Unpin + 'static, P> Service for OpensslAcceptorService<T, P> {
    type Request = Io<T, P>;
    type Response = Io<SslStream<T>, P>;
    type Error = HandshakeError<T>;
    type Future = OpensslAcceptorServiceFut<T, P>;

    fn poll_ready(
        self: Pin<&mut Self>,
        ctx: &mut Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        unimplemented!()
    }

    /*
    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        if self.conns.available() {
            Ok(Async::Ready(()))
        } else {
            Ok(Async::NotReady)
        }
    }
    */

    fn call(&mut self, req: Self::Request) -> Self::Future {
        let (io, params, _) = req.into_parts();
        let acc = self.acceptor.clone();
        OpensslAcceptorServiceFut {
            _guard: self.conns.get(),
            fut: async move {
                let acc = acc;
                tokio_openssl::accept(&acc, io).await
            }
                .boxed_local::<'static>(),
            params: Some(params),
        }
    }
}

pub struct OpensslAcceptorServiceFut<T, P>
where
    T: AsyncRead + AsyncWrite,
{
    fut: LocalBoxFuture<'static, Result<SslStream<T>, HandshakeError<T>>>,
    params: Option<P>,
    _guard: CounterGuard,
}

impl<T: AsyncRead + AsyncWrite, P> Future for OpensslAcceptorServiceFut<T, P> {
    type Output = Result<Io<SslStream<T>, P>, HandshakeError<T>>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unimplemented!()
    }

    /*
    type Item = Io<SslStream<T>, P>;
    type Error = HandshakeError<T>;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let io = futures::ready!(self.fut.poll())?;
        let proto = if let Some(protos) = io.get_ref().ssl().selected_alpn_protocol() {
            const H2: &[u8] = b"\x02h2";
            const HTTP10: &[u8] = b"\x08http/1.0";
            const HTTP11: &[u8] = b"\x08http/1.1";

            if protos.windows(3).any(|window| window == H2) {
                Protocol::Http2
            } else if protos.windows(9).any(|window| window == HTTP11) {
                Protocol::Http11
            } else if protos.windows(9).any(|window| window == HTTP10) {
                Protocol::Http10
            } else {
                Protocol::Unknown
            }
        } else {
            Protocol::Unknown
        };
        Ok(Async::Ready(Io::from_parts(
            io,
            self.params.take().unwrap(),
            proto,
        )))
    }
    */
}
