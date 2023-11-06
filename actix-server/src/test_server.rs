use std::{io, net, sync::mpsc, thread};

use actix_rt::{net::TcpStream, System};

use crate::{Server, ServerBuilder, ServerHandle, ServerServiceFactory};

/// A testing server.
///
/// `TestServer` is very simple test server that simplify process of writing integration tests for
/// network applications.
///
/// # Examples
/// ```
/// use actix_service::fn_service;
/// use actix_server::TestServer;
///
/// #[actix_rt::main]
/// async fn main() {
///     let srv = TestServer::start(|| fn_service(
///         |sock| async move {
///             println!("New connection: {:?}", sock);
///             Ok::<_, ()>(())
///         }
///     ));
///
///     println!("SOCKET: {:?}", srv.connect());
/// }
/// ```
pub struct TestServer;

/// Test server handle.
pub struct TestServerHandle {
    addr: net::SocketAddr,
    host: String,
    port: u16,
    server_handle: ServerHandle,
    thread_handle: Option<thread::JoinHandle<io::Result<()>>>,
}

impl TestServer {
    /// Start new `TestServer` using application factory and default server config.
    pub fn start(factory: impl ServerServiceFactory<TcpStream>) -> TestServerHandle {
        Self::start_with_builder(Server::build(), factory)
    }

    /// Start new `TestServer` using application factory and server builder.
    pub fn start_with_builder(
        server_builder: ServerBuilder,
        factory: impl ServerServiceFactory<TcpStream>,
    ) -> TestServerHandle {
        let (tx, rx) = mpsc::channel();

        // run server in separate thread
        let thread_handle = thread::spawn(move || {
            let lst = net::TcpListener::bind("127.0.0.1:0").unwrap();
            let local_addr = lst.local_addr().unwrap();

            System::new().block_on(async {
                let server = server_builder
                    .listen("test", lst, factory)
                    .unwrap()
                    .workers(1)
                    .disable_signals()
                    .run();

                tx.send((server.handle(), local_addr)).unwrap();
                server.await
            })
        });

        let (server_handle, addr) = rx.recv().unwrap();

        let host = format!("{}", addr.ip());
        let port = addr.port();

        TestServerHandle {
            addr,
            host,
            port,
            server_handle,
            thread_handle: Some(thread_handle),
        }
    }

    /// Get first available unused local address.
    pub fn unused_addr() -> net::SocketAddr {
        use socket2::{Domain, Protocol, Socket, Type};

        let addr: net::SocketAddr = "127.0.0.1:0".parse().unwrap();
        let domain = Domain::for_address(addr);
        let socket = Socket::new(domain, Type::STREAM, Some(Protocol::TCP)).unwrap();

        socket.set_reuse_address(true).unwrap();
        socket.set_nonblocking(true).unwrap();
        socket.bind(&addr.into()).unwrap();
        socket.listen(1024).unwrap();

        net::TcpListener::from(socket).local_addr().unwrap()
    }
}

impl TestServerHandle {
    /// Test server host.
    pub fn host(&self) -> &str {
        &self.host
    }

    /// Test server port.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Get test server address.
    pub fn addr(&self) -> net::SocketAddr {
        self.addr
    }

    /// Stop server.
    fn stop(&mut self) {
        drop(self.server_handle.stop(false));
        self.thread_handle.take().unwrap().join().unwrap().unwrap();
    }

    /// Connect to server, returning a Tokio `TcpStream`.
    pub fn connect(&self) -> io::Result<TcpStream> {
        TcpStream::from_std(net::TcpStream::connect(self.addr)?)
    }
}

impl Drop for TestServerHandle {
    fn drop(&mut self) {
        self.stop()
    }
}

#[cfg(test)]
mod tests {
    use actix_service::fn_service;

    use super::*;

    #[tokio::test]
    async fn connect_in_tokio_runtime() {
        let srv = TestServer::start(|| fn_service(|_sock| async move { Ok::<_, ()>(()) }));
        assert!(srv.connect().is_ok());
    }

    #[actix_rt::test]
    async fn connect_in_actix_runtime() {
        let srv = TestServer::start(|| fn_service(|_sock| async move { Ok::<_, ()>(()) }));
        assert!(srv.connect().is_ok());
    }
}
