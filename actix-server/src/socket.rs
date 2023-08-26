pub(crate) use std::net::{
    SocketAddr as StdSocketAddr, TcpListener as StdTcpListener, ToSocketAddrs,
};
use std::{fmt, io};

use actix_rt::net::TcpStream;
pub(crate) use mio::net::TcpListener as MioTcpListener;
use mio::{event::Source, Interest, Registry, Token};
#[cfg(unix)]
pub(crate) use {
    mio::net::UnixListener as MioUnixListener, std::os::unix::net::UnixListener as StdUnixListener,
};

use crate::builder::MpTcp;

pub(crate) enum MioListener {
    Tcp(MioTcpListener),
    #[cfg(unix)]
    Uds(MioUnixListener),
}

impl MioListener {
    pub(crate) fn local_addr(&self) -> SocketAddr {
        match *self {
            MioListener::Tcp(ref lst) => lst
                .local_addr()
                .map(SocketAddr::Tcp)
                .unwrap_or(SocketAddr::Unknown),
            #[cfg(unix)]
            MioListener::Uds(ref lst) => lst
                .local_addr()
                .map(SocketAddr::Uds)
                .unwrap_or(SocketAddr::Unknown),
        }
    }

    pub(crate) fn accept(&self) -> io::Result<MioStream> {
        match *self {
            MioListener::Tcp(ref lst) => lst.accept().map(|(stream, _)| MioStream::Tcp(stream)),
            #[cfg(unix)]
            MioListener::Uds(ref lst) => lst.accept().map(|(stream, _)| MioStream::Uds(stream)),
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
            #[cfg(unix)]
            MioListener::Uds(ref lst) => write!(f, "{:?}", lst),
        }
    }
}

impl fmt::Display for MioListener {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            MioListener::Tcp(ref lst) => write!(f, "{:?}", lst),
            #[cfg(unix)]
            MioListener::Uds(ref lst) => write!(f, "{:?}", lst),
        }
    }
}

pub(crate) enum SocketAddr {
    Unknown,
    Tcp(StdSocketAddr),
    #[cfg(unix)]
    Uds(mio::net::SocketAddr),
}

impl fmt::Display for SocketAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Unknown => write!(f, "Unknown SocketAddr"),
            Self::Tcp(ref addr) => write!(f, "{}", addr),
            #[cfg(unix)]
            Self::Uds(ref addr) => write!(f, "{:?}", addr),
        }
    }
}

impl fmt::Debug for SocketAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Unknown => write!(f, "Unknown SocketAddr"),
            Self::Tcp(ref addr) => write!(f, "{:?}", addr),
            #[cfg(unix)]
            Self::Uds(ref addr) => write!(f, "{:?}", addr),
        }
    }
}

#[derive(Debug)]
pub enum MioStream {
    Tcp(mio::net::TcpStream),
    #[cfg(unix)]
    Uds(mio::net::UnixStream),
}

/// Helper trait for converting a Mio stream into a Tokio stream.
pub trait FromStream: Sized {
    fn from_mio(sock: MioStream) -> io::Result<Self>;
}

#[cfg(windows)]
mod win_impl {
    use std::os::windows::io::{FromRawSocket, IntoRawSocket};

    use super::*;

    // TODO: This is a workaround and we need an efficient way to convert between Mio and Tokio stream
    impl FromStream for TcpStream {
        fn from_mio(sock: MioStream) -> io::Result<Self> {
            match sock {
                MioStream::Tcp(mio) => {
                    let raw = IntoRawSocket::into_raw_socket(mio);
                    // SAFETY: This is an in-place conversion from Mio stream to Tokio stream.
                    TcpStream::from_std(unsafe { FromRawSocket::from_raw_socket(raw) })
                }
            }
        }
    }
}

#[cfg(unix)]
mod unix_impl {
    use std::os::unix::io::{FromRawFd, IntoRawFd};

    use actix_rt::net::UnixStream;

    use super::*;

    // HACK: This is a workaround and we need an efficient way to convert between Mio and Tokio stream
    impl FromStream for TcpStream {
        fn from_mio(sock: MioStream) -> io::Result<Self> {
            match sock {
                MioStream::Tcp(mio) => {
                    let raw = IntoRawFd::into_raw_fd(mio);
                    // SAFETY: This is an in-place conversion from Mio stream to Tokio stream.
                    TcpStream::from_std(unsafe { FromRawFd::from_raw_fd(raw) })
                }
                MioStream::Uds(_) => {
                    panic!("Should not happen, bug in server impl");
                }
            }
        }
    }

    // HACK: This is a workaround and we need an efficient way to convert between Mio and Tokio stream
    impl FromStream for UnixStream {
        fn from_mio(sock: MioStream) -> io::Result<Self> {
            match sock {
                MioStream::Tcp(_) => panic!("Should not happen, bug in server impl"),
                MioStream::Uds(mio) => {
                    let raw = IntoRawFd::into_raw_fd(mio);
                    // SAFETY: This is an in-place conversion from Mio stream to Tokio stream.
                    UnixStream::from_std(unsafe { FromRawFd::from_raw_fd(raw) })
                }
            }
        }
    }
}

pub(crate) fn create_mio_tcp_listener(
    addr: StdSocketAddr,
    backlog: u32,
    mptcp: &MpTcp,
) -> io::Result<MioTcpListener> {
    use socket2::{Domain, Protocol, Socket, Type};

    #[cfg(not(target_os = "linux"))]
    let protocol = Protocol::TCP;
    #[cfg(target_os = "linux")]
    let protocol = if matches!(mptcp, MpTcp::Disabled) {
        Protocol::TCP
    } else {
        Protocol::MPTCP
    };

    let socket = match Socket::new(Domain::for_address(addr), Type::STREAM, Some(protocol)) {
        Ok(sock) => sock,

        Err(err) if matches!(mptcp, MpTcp::TcpFallback) => {
            tracing::warn!("binding socket as MPTCP failed: {err}");
            tracing::warn!("falling back to TCP");
            Socket::new(Domain::for_address(addr), Type::STREAM, Some(Protocol::TCP))?
        }

        Err(err) => return Err(err),
    };

    socket.set_reuse_address(true)?;
    socket.set_nonblocking(true)?;
    socket.bind(&addr.into())?;
    socket.listen(backlog as i32)?;

    Ok(MioTcpListener::from_std(StdTcpListener::from(socket)))
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
        let lst = create_mio_tcp_listener(addr, 128, &MpTcp::Disabled).unwrap();
        let lst = MioListener::Tcp(lst);
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
