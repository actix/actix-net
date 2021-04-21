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
    Tcp(actix_rt::net::TcpStream),
    #[cfg(unix)]
    Uds(actix_rt::net::UnixStream),
}

/// helper trait for converting mio stream to tokio stream.
pub trait FromStream: Sized {
    fn from_stream(stream: Stream) -> Self;
}

#[cfg(windows)]
mod win_impl {
    use super::*;

    use std::os::windows::io::{FromRawSocket, IntoRawSocket};

    // FIXME: This is a workaround and we need an efficient way to convert between mio and tokio stream
    impl FromStream for TcpStream {
        fn from_stream(stream: Stream) -> Self {
            match stream {
                MioStream::Tcp(stream) => stream,
            }
        }
    }
}

#[cfg(unix)]
mod unix_impl {
    use super::*;

    // FIXME: This is a workaround and we need an efficient way to convert between mio and tokio stream
    impl FromStream for TcpStream {
        fn from_stream(stream: Stream) -> Self {
            match stream {
                Stream::Tcp(stream) => stream,
                Stream::Uds(_) => {
                    panic!("Should not happen, bug in server impl");
                }
            }
        }
    }

    // FIXME: This is a workaround and we need an efficient way to convert between mio and tokio stream
    impl FromStream for actix_rt::net::UnixStream {
        fn from_stream(stream: Stream) -> Self {
            match stream {
                Stream::Tcp(_) => panic!("Should not happen, bug in server impl"),
                Stream::Uds(stream) => stream,
            }
        }
    }
}

// #[cfg(test)]
// mod tests {
//     use super::*;

//     #[test]
//     fn socket_addr() {
//         let addr = SocketAddr::Tcp("127.0.0.1:8080".parse().unwrap());
//         assert!(format!("{:?}", addr).contains("127.0.0.1:8080"));
//         assert_eq!(format!("{}", addr), "127.0.0.1:8080");

//         let addr: StdSocketAddr = "127.0.0.1:0".parse().unwrap();
//         let socket = MioTcpSocket::new_v4().unwrap();
//         socket.set_reuseaddr(true).unwrap();
//         socket.bind(addr).unwrap();
//         let tcp = socket.listen(128).unwrap();
//         let lst = Listener::Tcp(tcp);
//         assert!(format!("{:?}", lst).contains("TcpListener"));
//         assert!(format!("{}", lst).contains("127.0.0.1"));
//     }

//     #[test]
//     #[cfg(unix)]
//     fn uds() {
//         let _ = std::fs::remove_file("/tmp/sock.xxxxx");
//         if let Ok(socket) = MioUnixListener::bind("/tmp/sock.xxxxx") {
//             let addr = socket.local_addr().expect("Couldn't get local address");
//             let a = SocketAddr::Uds(addr);
//             assert!(format!("{:?}", a).contains("/tmp/sock.xxxxx"));
//             assert!(format!("{}", a).contains("/tmp/sock.xxxxx"));

//             let lst = Listener::Uds(socket);
//             assert!(format!("{:?}", lst).contains("/tmp/sock.xxxxx"));
//             assert!(format!("{}", lst).contains("/tmp/sock.xxxxx"));
//         }
//     }
// }
