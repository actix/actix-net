use std::net::{SocketAddr as StdTcpSocketAddr, TcpListener as StdTcpListener};
use std::{fmt, io};

#[cfg(unix)]
use std::os::unix::{
    io::{FromRawFd, IntoRawFd},
    net::{SocketAddr as StdUdsSocketAddr, UnixListener as StdUnixListener},
};
#[cfg(windows)]
use std::os::windows::io::{FromRawSocket, IntoRawSocket};

use actix_rt::net::TcpStream;
#[cfg(unix)]
use actix_rt::net::UnixStream;
use mio::event::Source;
#[cfg(unix)]
use mio::net::{
    SocketAddr as MioSocketAddr, UnixListener as MioUnixListener, UnixStream as MioUnixStream,
};
use mio::net::{TcpListener as MioTcpListener, TcpStream as MioTcpStream};
use mio::{Interest, Registry, Token};

/// socket module contains a unified wrapper for Tcp/Uds listener/SocketAddr/Stream and necessary
/// trait impl for registering the listener to mio::Poll and convert stream to
/// `actix_rt::net::{TcpStream, UnixStream}`.

pub(crate) enum StdListener {
    Tcp(StdTcpListener),
    #[cfg(unix)]
    Uds(StdUnixListener),
}

pub(crate) enum SocketAddr {
    Tcp(StdTcpSocketAddr),
    #[cfg(unix)]
    Uds(StdUdsSocketAddr),
    // this is a work around. mio would return different types of SocketAddr between accept and
    // local_addr methods.
    #[cfg(unix)]
    UdsMio(MioSocketAddr),
}

impl fmt::Display for SocketAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            SocketAddr::Tcp(ref addr) => write!(f, "{}", addr),
            #[cfg(unix)]
            SocketAddr::Uds(ref addr) => write!(f, "{:?}", addr),
            #[cfg(unix)]
            SocketAddr::UdsMio(ref addr) => write!(f, "{:?}", addr),
        }
    }
}

impl fmt::Debug for SocketAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            SocketAddr::Tcp(ref addr) => write!(f, "{:?}", addr),
            #[cfg(unix)]
            SocketAddr::Uds(ref addr) => write!(f, "{:?}", addr),
            #[cfg(unix)]
            SocketAddr::UdsMio(ref addr) => write!(f, "{:?}", addr),
        }
    }
}

impl fmt::Display for StdListener {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            StdListener::Tcp(ref lst) => write!(f, "{}", lst.local_addr().ok().unwrap()),
            #[cfg(unix)]
            StdListener::Uds(ref lst) => write!(f, "{:?}", lst.local_addr().ok().unwrap()),
        }
    }
}

impl StdListener {
    pub(crate) fn local_addr(&self) -> SocketAddr {
        match self {
            StdListener::Tcp(lst) => SocketAddr::Tcp(lst.local_addr().unwrap()),
            #[cfg(unix)]
            StdListener::Uds(lst) => SocketAddr::Uds(lst.local_addr().unwrap()),
        }
    }

    pub(crate) fn into_mio_listener(self) -> std::io::Result<MioSocketListener> {
        match self {
            StdListener::Tcp(lst) => {
                // ToDo: is this non_blocking a good practice?
                lst.set_nonblocking(true)?;
                Ok(MioSocketListener::Tcp(MioTcpListener::from_std(lst)))
            }
            #[cfg(unix)]
            StdListener::Uds(lst) => {
                // ToDo: the same as above
                lst.set_nonblocking(true)?;
                Ok(MioSocketListener::Uds(MioUnixListener::from_std(lst)))
            }
        }
    }
}

#[derive(Debug)]
pub enum MioStream {
    Tcp(MioTcpStream),
    #[cfg(unix)]
    Uds(MioUnixStream),
}

pub(crate) enum MioSocketListener {
    Tcp(MioTcpListener),
    #[cfg(unix)]
    Uds(MioUnixListener),
}

impl MioSocketListener {
    pub(crate) fn accept(&self) -> io::Result<Option<(MioStream, SocketAddr)>> {
        match *self {
            MioSocketListener::Tcp(ref lst) => lst
                .accept()
                .map(|(stream, addr)| Some((MioStream::Tcp(stream), SocketAddr::Tcp(addr)))),
            #[cfg(unix)]
            MioSocketListener::Uds(ref lst) => lst
                .accept()
                .map(|(stream, addr)| Some((MioStream::Uds(stream), SocketAddr::UdsMio(addr)))),
        }
    }
}

impl Source for MioSocketListener {
    fn register(
        &mut self,
        registry: &Registry,
        token: Token,
        interests: Interest,
    ) -> io::Result<()> {
        match *self {
            MioSocketListener::Tcp(ref mut lst) => lst.register(registry, token, interests),
            #[cfg(unix)]
            MioSocketListener::Uds(ref mut lst) => lst.register(registry, token, interests),
        }
    }

    fn reregister(
        &mut self,
        registry: &Registry,
        token: Token,
        interests: Interest,
    ) -> io::Result<()> {
        match *self {
            MioSocketListener::Tcp(ref mut lst) => lst.reregister(registry, token, interests),
            #[cfg(unix)]
            MioSocketListener::Uds(ref mut lst) => lst.reregister(registry, token, interests),
        }
    }

    fn deregister(&mut self, registry: &Registry) -> io::Result<()> {
        match *self {
            MioSocketListener::Tcp(ref mut lst) => lst.deregister(registry),
            #[cfg(unix)]
            MioSocketListener::Uds(ref mut lst) => {
                let res = lst.deregister(registry);

                // cleanup file path
                if let Ok(addr) = lst.local_addr() {
                    if let Some(path) = addr.as_pathname() {
                        let _ = std::fs::remove_file(path);
                    }
                }
                res
            }
        }
    }
}

/// helper trait for converting mio stream to tokio stream.
pub trait FromStream: Sized {
    fn from_mio(sock: MioStream) -> io::Result<Self>;
}

// ToDo: This is a workaround and we need an efficient way to convert between mio and tokio stream
#[cfg(unix)]
impl FromStream for TcpStream {
    fn from_mio(sock: MioStream) -> io::Result<Self> {
        match sock {
            MioStream::Tcp(mio) => {
                let raw = IntoRawFd::into_raw_fd(mio);
                // # Safety:
                // This is a in place conversion from mio stream to tokio stream.
                TcpStream::from_std(unsafe { FromRawFd::from_raw_fd(raw) })
            }
            MioStream::Uds(_) => {
                panic!("Should not happen, bug in server impl");
            }
        }
    }
}

// ToDo: This is a workaround and we need an efficient way to convert between mio and tokio stream
#[cfg(windows)]
impl FromStream for TcpStream {
    fn from_mio(sock: MioStream) -> io::Result<Self> {
        match sock {
            MioStream::Tcp(mio) => {
                let raw = IntoRawSocket::into_raw_socket(mio);
                // # Safety:
                // This is a in place conversion from mio stream to tokio stream.
                TcpStream::from_std(unsafe { FromRawSocket::from_raw_socket(raw) })
            }
        }
    }
}

// ToDo: This is a workaround and we need an efficient way to convert between mio and tokio stream
#[cfg(unix)]
impl FromStream for UnixStream {
    fn from_mio(sock: MioStream) -> io::Result<Self> {
        match sock {
            MioStream::Tcp(_) => panic!("Should not happen, bug in server impl"),
            MioStream::Uds(mio) => {
                let raw = IntoRawFd::into_raw_fd(mio);
                // # Safety:
                // This is a in place conversion from mio stream to tokio stream.
                UnixStream::from_std(unsafe { FromRawFd::from_raw_fd(raw) })
            }
        }
    }
}
