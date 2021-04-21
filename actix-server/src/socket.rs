pub(crate) use std::net::{
    SocketAddr as StdSocketAddr, TcpListener as StdTcpListener, ToSocketAddrs,
};

pub(crate) use actix_rt::net::{TcpListener, TcpSocket};
#[cfg(unix)]
pub(crate) use {
    actix_rt::net::UnixListener, std::os::unix::net::UnixListener as StdUnixListener,
};

use std::{
    fmt, io,
    task::{Context, Poll},
};

use actix_rt::net::TcpStream;

pub(crate) enum Listener {
    Tcp(TcpListener),
    #[cfg(unix)]
    Uds(tokio::net::UnixListener),
}

impl Listener {
    pub(crate) fn poll_accept(&self, cx: &mut Context<'_>) -> Poll<io::Result<Stream>> {
        match *self {
            Self::Tcp(ref lst) => lst
                .poll_accept(cx)
                .map_ok(|(stream, _)| Stream::Tcp(stream)),
            #[cfg(unix)]
            Self::Uds(ref lst) => lst
                .poll_accept(cx)
                .map_ok(|(stream, _)| Stream::Uds(stream)),
        }
    }
}

// TODO: use TryFrom
impl From<StdTcpListener> for Listener {
    fn from(lst: StdTcpListener) -> Self {
        let lst = TcpListener::from_std(lst).unwrap();
        Listener::Tcp(lst)
    }
}

#[cfg(unix)]
impl From<StdUnixListener> for Listener {
    fn from(lst: StdUnixListener) -> Self {
        let lst = UnixListener::from_std(lst).unwrap();
        Listener::Uds(lst)
    }
}

impl fmt::Debug for Listener {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Listener::Tcp(ref lst) => write!(f, "{:?}", lst),
            #[cfg(all(unix))]
            Listener::Uds(ref lst) => write!(f, "{:?}", lst),
        }
    }
}

impl fmt::Display for Listener {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Listener::Tcp(ref lst) => write!(f, "{}", lst.local_addr().ok().unwrap()),
            #[cfg(unix)]
            Listener::Uds(ref lst) => write!(f, "{:?}", lst.local_addr().ok().unwrap()),
        }
    }
}

#[derive(Debug)]
pub enum Stream {
    Tcp(TcpStream),
    #[cfg(unix)]
    Uds(actix_rt::net::UnixStream),
}

/// helper trait for converting mio stream to tokio stream.
pub trait FromStream: Sized {
    fn from_stream(stream: Stream) -> io::Result<Self>;
}

#[cfg(windows)]
mod win_impl {
    use super::*;

    impl FromStream for TcpStream {
        fn from_stream(stream: Stream) -> io::Result<Self> {
            match stream {
                Stream::Tcp(stream) => {
                    let stream = stream.into_std()?;
                    TcpStream::from_std(stream)
                }
            }
        }
    }
}

#[cfg(unix)]
mod unix_impl {
    use super::*;

    use actix_rt::net::UnixStream;

    impl FromStream for TcpStream {
        fn from_stream(stream: Stream) -> io::Result<Self> {
            match stream {
                Stream::Tcp(stream) => {
                    let stream = stream.into_std()?;
                    TcpStream::from_std(stream)
                }
                Stream::Uds(_) => {
                    panic!("Should not happen, bug in server impl");
                }
            }
        }
    }

    impl FromStream for UnixStream {
        fn from_stream(stream: Stream) -> io::Result<Self> {
            match stream {
                Stream::Tcp(_) => panic!("Should not happen, bug in server impl"),
                Stream::Uds(stream) => {
                    let stream = stream.into_std()?;
                    UnixStream::from_std(stream)
                }
            }
        }
    }
}
