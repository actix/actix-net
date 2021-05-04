pub(crate) use std::net::{
    SocketAddr as StdSocketAddr, TcpListener as StdTcpListener, ToSocketAddrs,
};

pub(crate) use mio::net::{TcpListener as MioTcpListener, TcpSocket as MioTcpSocket};
#[cfg(unix)]
pub(crate) use {
    mio::net::UnixListener as MioUnixListener,
    std::os::unix::net::UnixListener as StdUnixListener,
};

use std::{fmt, io};

use actix_rt::net::TcpStream;
use mio::net::TcpStream as MioTcpStream;
use mio::{event::Source, Interest, Registry, Token};

use crate::accept::{AcceptContext, Acceptable};

/// impl Acceptable trait for [mio::net::TcpListener] so it can be managed by server and it's [mio::Poll] instance.
impl Acceptable for MioTcpListener {
    type Connection = MioTcpStream;

    fn accept(
        &mut self,
        _: &mut AcceptContext<'_, Self::Connection>,
    ) -> io::Result<Option<Self::Connection>> {
        Self::accept(self).map(|stream| Some(stream.0))
    }

    fn register(&mut self, registry: &mio::Registry, token: mio::Token) -> io::Result<()> {
        Source::register(self, registry, token, Interest::READABLE)
    }

    fn deregister(&mut self, registry: &mio::Registry) -> io::Result<()> {
        Source::deregister(self, registry)
    }
}

pub enum MioListener {
    Tcp(MioTcpListener),
    #[cfg(unix)]
    Uds(MioUnixListener),
}

impl From<StdTcpListener> for MioListener {
    fn from(lst: StdTcpListener) -> Self {
        MioListener::Tcp(MioTcpListener::from_std(lst))
    }
}

impl Acceptable for MioListener {
    type Connection = MioStream;

    fn accept(
        &mut self,
        _: &mut AcceptContext<'_, Self::Connection>,
    ) -> io::Result<Option<Self::Connection>> {
        match *self {
            MioListener::Tcp(ref mut lst) => {
                MioTcpListener::accept(lst).map(|stream| Some(MioStream::Tcp(stream.0)))
            }
            #[cfg(unix)]
            MioListener::Uds(ref mut lst) => {
                MioUnixListener::accept(lst).map(|stream| Some(MioStream::Uds(stream.0)))
            }
        }
    }

    fn register(&mut self, registry: &Registry, token: Token) -> io::Result<()> {
        match *self {
            MioListener::Tcp(ref mut lst) => {
                Source::register(lst, registry, token, Interest::READABLE)
            }
            #[cfg(unix)]
            MioListener::Uds(ref mut lst) => {
                Source::register(lst, registry, token, Interest::READABLE)
            }
        }
    }

    fn deregister(&mut self, registry: &Registry) -> io::Result<()> {
        match *self {
            MioListener::Tcp(ref mut lst) => Source::deregister(lst, registry),
            #[cfg(unix)]
            MioListener::Uds(ref mut lst) => {
                let res = Source::deregister(lst, registry);

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

impl fmt::Debug for MioListener {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            MioListener::Tcp(ref lst) => write!(f, "{:?}", lst),
            #[cfg(all(unix))]
            MioListener::Uds(ref lst) => write!(f, "{:?}", lst),
        }
    }
}

#[derive(Debug)]
pub enum MioStream {
    Tcp(MioTcpStream),
    #[cfg(unix)]
    Uds(mio::net::UnixStream),
}

/// helper trait for converting mio stream to tokio stream.
pub trait FromConnection<C>: Sized {
    fn from_conn(conn: C) -> io::Result<Self>;
}

#[cfg(windows)]
mod win_impl {
    use super::*;

    use std::os::windows::io::{FromRawSocket, IntoRawSocket};

    // FIXME: This is a workaround and we need an efficient way to convert between mio and tokio stream
    impl FromConnection<MioTcpStream> for TcpStream {
        fn from_conn(conn: MioTcpStream) -> io::Result<Self> {
            let raw = IntoRawSocket::into_raw_socket(conn);
            // SAFETY: This is a in place conversion from mio stream to tokio stream.
            TcpStream::from_std(unsafe { FromRawSocket::from_raw_socket(raw) })
        }
    }

    impl FromConnection<MioStream> for TcpStream {
        fn from_conn(stream: MioStream) -> io::Result<Self> {
            match stream {
                MioStream::Tcp(tcp) => FromConnection::from_conn(tcp),
            }
        }
    }
}

#[cfg(unix)]
mod unix_impl {
    use super::*;

    use std::os::unix::io::{FromRawFd, IntoRawFd};

    use actix_rt::net::UnixStream;
    use mio::net::UnixStream as MioUnixStream;

    impl From<StdUnixListener> for MioListener {
        fn from(lst: StdUnixListener) -> Self {
            MioListener::Uds(MioUnixListener::from_std(lst))
        }
    }

    /// impl Acceptable trait for [mio::net::UnixListener] so it can be managed by server and it's [mio::Poll] instance.
    impl Acceptable for MioUnixListener {
        type Connection = MioUnixStream;

        fn accept(
            &mut self,
            _: &mut AcceptContext<'_, Self::Connection>,
        ) -> io::Result<Option<Self::Connection>> {
            Self::accept(self).map(|stream| Some(stream.0))
        }

        fn register(&mut self, registry: &mio::Registry, token: mio::Token) -> io::Result<()> {
            Source::register(self, registry, token, Interest::READABLE)
        }

        fn deregister(&mut self, registry: &mio::Registry) -> io::Result<()> {
            Source::deregister(self, registry)
        }
    }

    // FIXME: This is a workaround and we need an efficient way to convert between mio and tokio stream
    impl FromConnection<MioTcpStream> for TcpStream {
        fn from_conn(conn: MioTcpStream) -> io::Result<Self> {
            let raw = IntoRawFd::into_raw_fd(conn);
            // SAFETY: This is a in place conversion from mio stream to tokio stream.
            TcpStream::from_std(unsafe { FromRawFd::from_raw_fd(raw) })
        }
    }

    // FIXME: This is a workaround and we need an efficient way to convert between mio and tokio stream
    impl FromConnection<MioUnixStream> for UnixStream {
        fn from_conn(conn: MioUnixStream) -> io::Result<Self> {
            let raw = IntoRawFd::into_raw_fd(conn);
            // SAFETY: This is a in place conversion from mio stream to tokio stream.
            UnixStream::from_std(unsafe { FromRawFd::from_raw_fd(raw) })
        }
    }

    impl FromConnection<MioStream> for TcpStream {
        fn from_conn(stream: MioStream) -> io::Result<Self> {
            match stream {
                MioStream::Tcp(tcp) => FromConnection::from_conn(tcp),
                MioStream::Uds(_) => unreachable!("UnixStream can not convert to TcpStream"),
            }
        }
    }

    impl FromConnection<MioStream> for UnixStream {
        fn from_conn(stream: MioStream) -> io::Result<Self> {
            match stream {
                MioStream::Tcp(_) => unreachable!("TcpStream can not convert to UnixStream"),
                MioStream::Uds(uds) => FromConnection::from_conn(uds),
            }
        }
    }
}
