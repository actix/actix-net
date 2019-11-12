use std::convert::Infallible;
use std::future::Future;
use std::io;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};

use actix_service::{Service, ServiceFactory};
use futures::future;
use native_tls::{Error, HandshakeError, TlsAcceptor, TlsStream};
use tokio_io::{AsyncRead, AsyncWrite};

use crate::counter::{Counter, CounterGuard};
use crate::ssl::MAX_CONN_COUNTER;
use crate::{Io, Protocol, ServerConfig};

/// Support `SSL` connections via native-tls package
///
/// `tls` feature enables `NativeTlsAcceptor` type
pub struct NativeTlsAcceptor<T, P = ()> {
    acceptor: TlsAcceptor,
    io: PhantomData<(T, P)>,
}

impl<T: AsyncRead + AsyncWrite, P> NativeTlsAcceptor<T, P> {
    /// Create `NativeTlsAcceptor` instance
    pub fn new(acceptor: TlsAcceptor) -> Self {
        NativeTlsAcceptor {
            acceptor,
            io: PhantomData,
        }
    }
}

impl<T, P> Clone for NativeTlsAcceptor<T, P> {
    fn clone(&self) -> Self {
        Self {
            acceptor: self.acceptor.clone(),
            io: PhantomData,
        }
    }
}

impl<T: AsyncRead + AsyncWrite, P> ServiceFactory for NativeTlsAcceptor<T, P> {
    type Request = Io<T, P>;
    type Response = Io<NativeTlsStream<T>, P>;
    type Error = Error;

    type Config = ServerConfig;
    type Service = NativeTlsAcceptorService<T, P>;
    type InitError = Infallible;
    type Future = future::Ready<Result<Self::Service, Self::InitError>>;

    fn new_service(&self, cfg: &ServerConfig) -> Self::Future {
        cfg.set_secure();

        MAX_CONN_COUNTER.with(|conns| {
            future::ok(NativeTlsAcceptorService {
                acceptor: self.acceptor.clone(),
                conns: conns.clone(),
                io: PhantomData,
            })
        })
    }
}

pub struct NativeTlsAcceptorService<T, P> {
    acceptor: TlsAcceptor,
    io: PhantomData<(T, P)>,
    conns: Counter,
}

impl<T: AsyncRead + AsyncWrite, P> Service for NativeTlsAcceptorService<T, P> {
    type Request = Io<T, P>;
    type Response = Io<NativeTlsStream<T>, P>;
    type Error = Error;
    type Future = Accept<T, P>;

    fn poll_ready(&mut self, ctx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        if self.conns.available(ctx) {
            Ok(Poll::Ready(Ok(())))
        } else {
            Ok(Poll::Pending)
        }
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        let (io, params, _) = req.into_parts();
        Accept {
            _guard: self.conns.get(),
            inner: Some(self.acceptor.accept(io)),
            params: Some(params),
        }
    }
}

/// Future returned from `NativeTlsAcceptor::accept` which will resolve
/// once the accept handshake has finished.
pub struct Accept<S, P> {
    inner: Option<Result<TlsStream<S>, HandshakeError<S>>>,
    params: Option<P>,
    _guard: CounterGuard,
}

impl<T: AsyncRead + AsyncWrite, P> Future for Accept<T, P> {
    type Output = Result<Io<NativeTlsStream<T>, P>, Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let my = self.get_mut();
        match my.inner.take().expect("cannot poll MidHandshake twice") {
            Ok(stream) => Poll::Ready(Ok(Io::from_parts(
                NativeTlsStream { inner: stream },
                my.params.take().unwrap(),
                Protocol::Unknown,
            ))),
            Err(HandshakeError::Failure(e)) => Poll::Ready(Err(e)),
            Err(HandshakeError::WouldBlock(s)) => match s.handshake() {
                Ok(stream) => Poll::Ready(Ok(Io::from_parts(
                    NativeTlsStream { inner: stream },
                    my.params.take().unwrap(),
                    Protocol::Unknown,
                ))),
                Err(HandshakeError::Failure(e)) => Poll::Ready(Err(e)),
                Err(HandshakeError::WouldBlock(s)) => {
                    my.inner = Some(Err(HandshakeError::WouldBlock(s)));
                    // TODO: should we use Waker somehow?
                    Poll::Pending
                }
            },
        }
    }
}

/// A wrapper around an underlying raw stream which implements the TLS or SSL
/// protocol.
///
/// A `NativeTlsStream<S>` represents a handshake that has been completed successfully
/// and both the server and the client are ready for receiving and sending
/// data. Bytes read from a `NativeTlsStream` are decrypted from `S` and bytes written
/// to a `NativeTlsStream` are encrypted when passing through to `S`.
#[derive(Debug)]
pub struct NativeTlsStream<S> {
    inner: TlsStream<S>,
}

impl<S> AsRef<TlsStream<S>> for NativeTlsStream<S> {
    fn as_ref(&self) -> &TlsStream<S> {
        &self.inner
    }
}

impl<S> AsMut<TlsStream<S>> for NativeTlsStream<S> {
    fn as_mut(&mut self) -> &mut TlsStream<S> {
        &mut self.inner
    }
}

impl<S: io::Read + io::Write> io::Read for NativeTlsStream<S> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}

impl<S: io::Read + io::Write> io::Write for NativeTlsStream<S> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inner.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

impl<S: AsyncRead + AsyncWrite> AsyncRead for NativeTlsStream<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        // TODO: wha?
        unimplemented!()
    }
}

impl<S: AsyncRead + AsyncWrite> AsyncWrite for NativeTlsStream<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        unimplemented!()
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        unimplemented!()
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), io::Error>> {
        let inner = &mut Pin::get_mut(self).inner;
        match inner.shutdown() {
            Ok(_) => (),
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => (),
            Err(e) => return Poll::Ready(Err(e)),
        }
        inner.get_mut().poll_shutdown()
    }
}
