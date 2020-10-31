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
use mio::event::Source;
use mio::net::TcpStream as MioTcpStream;
use mio::{Interest, Registry, Token};

#[cfg(windows)]
use std::os::windows::io::{FromRawSocket, IntoRawSocket};
#[cfg(unix)]
use {
    actix_rt::net::UnixStream,
    mio::net::{SocketAddr as MioSocketAddr, UnixStream as MioUnixStream},
    std::os::unix::io::{FromRawFd, IntoRawFd},
};

pub(crate) enum MioListener {
    Tcp(MioTcpListener),
    #[cfg(unix)]
    Uds(MioUnixListener),
}

impl MioListener {
    pub(crate) fn local_addr(&self) -> SocketAddr {
        match *self {
            MioListener::Tcp(ref lst) => SocketAddr::Tcp(lst.local_addr().unwrap()),
            #[cfg(unix)]
            MioListener::Uds(ref lst) => SocketAddr::Uds(lst.local_addr().unwrap()),
        }
    }

    pub(crate) fn accept(&self) -> io::Result<Option<(MioStream, SocketAddr)>> {
        match *self {
            MioListener::Tcp(ref lst) => lst
                .accept()
                .map(|(stream, addr)| Some((MioStream::Tcp(stream), SocketAddr::Tcp(addr)))),
            #[cfg(unix)]
            MioListener::Uds(ref lst) => lst
                .accept()
                .map(|(stream, addr)| Some((MioStream::Uds(stream), SocketAddr::Uds(addr)))),
        }
    }
}

impl Source for MioListener {
    fn register(
        &mut self,
        registry: &Registry,
        token: Token,
        interests: Interest,
    ) -> io::Result<()> {
        match *self {
            MioListener::Tcp(ref mut lst) => lst.register(registry, token, interests),
            #[cfg(unix)]
            MioListener::Uds(ref mut lst) => lst.register(registry, token, interests),
        }
    }

    fn reregister(
        &mut self,
        registry: &Registry,
        token: Token,
        interests: Interest,
    ) -> io::Result<()> {
        match *self {
            MioListener::Tcp(ref mut lst) => lst.reregister(registry, token, interests),
            #[cfg(unix)]
            MioListener::Uds(ref mut lst) => lst.reregister(registry, token, interests),
        }
    }

    fn deregister(&mut self, registry: &Registry) -> io::Result<()> {
        match *self {
            MioListener::Tcp(ref mut lst) => lst.deregister(registry),
            #[cfg(unix)]
            MioListener::Uds(ref mut lst) => {
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

impl From<StdTcpListener> for MioListener {
    fn from(lst: StdTcpListener) -> Self {
        MioListener::Tcp(MioTcpListener::from_std(lst))
    }
}

#[cfg(unix)]
impl From<StdUnixListener> for MioListener {
    fn from(lst: StdUnixListener) -> Self {
        MioListener::Uds(MioUnixListener::from_std(lst))
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

impl fmt::Display for MioListener {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            MioListener::Tcp(ref lst) => write!(f, "{}", lst.local_addr().ok().unwrap()),
            #[cfg(unix)]
            MioListener::Uds(ref lst) => write!(f, "{:?}", lst.local_addr().ok().unwrap()),
        }
    }
}

pub(crate) enum SocketAddr {
    Tcp(StdSocketAddr),
    #[cfg(unix)]
    Uds(MioSocketAddr),
}

impl fmt::Display for SocketAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            SocketAddr::Tcp(ref addr) => write!(f, "{}", addr),
            #[cfg(unix)]
            SocketAddr::Uds(ref addr) => write!(f, "{:?}", addr),
        }
    }
}

impl fmt::Debug for SocketAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            SocketAddr::Tcp(ref addr) => write!(f, "{:?}", addr),
            #[cfg(unix)]
            SocketAddr::Uds(ref addr) => write!(f, "{:?}", addr),
        }
    }
}

#[derive(Debug)]
pub enum MioStream {
    Tcp(MioTcpStream),
    #[cfg(unix)]
    Uds(MioUnixStream),
}

/// helper trait for converting mio stream to tokio stream.
pub trait FromStream: Sized {
    fn from_mio(sock: MioStream) -> io::Result<Self>;
}

// FIXME: This is a workaround and we need an efficient way to convert between mio and tokio stream
#[cfg(unix)]
impl FromStream for TcpStream {
    fn from_mio(sock: MioStream) -> io::Result<Self> {
        match sock {
            MioStream::Tcp(mio) => {
                let raw = IntoRawFd::into_raw_fd(mio);
                // SAFETY: This is a in place conversion from mio stream to tokio stream.
                TcpStream::from_std(unsafe { FromRawFd::from_raw_fd(raw) })
            }
            MioStream::Uds(_) => {
                panic!("Should not happen, bug in server impl");
            }
        }
    }
}

// FIXME: This is a workaround and we need an efficient way to convert between mio and tokio stream
#[cfg(windows)]
impl FromStream for TcpStream {
    fn from_mio(sock: MioStream) -> io::Result<Self> {
        match sock {
            MioStream::Tcp(mio) => {
                let raw = IntoRawSocket::into_raw_socket(mio);
                // SAFETY: This is a in place conversion from mio stream to tokio stream.
                TcpStream::from_std(unsafe { FromRawSocket::from_raw_socket(raw) })
            }
        }
    }
}

// FIXME: This is a workaround and we need an efficient way to convert between mio and tokio stream
#[cfg(unix)]
impl FromStream for UnixStream {
    fn from_mio(sock: MioStream) -> io::Result<Self> {
        match sock {
            MioStream::Tcp(_) => panic!("Should not happen, bug in server impl"),
            MioStream::Uds(mio) => {
                let raw = IntoRawFd::into_raw_fd(mio);
                // SAFETY: This is a in place conversion from mio stream to tokio stream.
                UnixStream::from_std(unsafe { FromRawFd::from_raw_fd(raw) })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_addr() {
        let addr = SocketAddr::Tcp("127.0.0.1:8080".parse().unwrap());
        assert!(format!("{:?}", addr).contains("127.0.0.1:8080"));
        assert_eq!(format!("{}", addr), "127.0.0.1:8080");

        let addr: StdSocketAddr = "127.0.0.1:0".parse().unwrap();
        let socket = MioTcpSocket::new_v4().unwrap();
        socket.set_reuseaddr(true).unwrap();
        socket.bind(addr).unwrap();
        let tcp = socket.listen(128).unwrap();
        let lst = MioListener::Tcp(tcp);
        assert!(format!("{:?}", lst).contains("TcpListener"));
        assert!(format!("{}", lst).contains("127.0.0.1"));
    }

    #[test]
    #[cfg(unix)]
    fn uds() {
        let _ = std::fs::remove_file("/tmp/sock.xxxxx");
        if let Ok(socket) = MioUnixListener::bind("/tmp/sock.xxxxx") {
            let addr = socket.local_addr().expect("Couldn't get local address");
            let a = SocketAddr::Uds(addr);
            assert!(format!("{:?}", a).contains("/tmp/sock.xxxxx"));
            assert!(format!("{}", a).contains("/tmp/sock.xxxxx"));

            let lst = MioListener::Uds(socket);
            assert!(format!("{:?}", lst).contains("/tmp/sock.xxxxx"));
            assert!(format!("{}", lst).contains("/tmp/sock.xxxxx"));
        }
    }
}
