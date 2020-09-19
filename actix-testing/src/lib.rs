//! Various helpers for Actix applications to use during testing.
#![deny(rust_2018_idioms, warnings)]
#![allow(clippy::type_complexity, clippy::needless_doctest_main)]

use std::sync::mpsc;
use std::{net, thread};

use actix_rt::{net::TcpStream, System};
use actix_server::{Server, ServerBuilder, ServiceFactory};
use socket2::{Domain, Protocol, Socket, Type};

#[cfg(not(test))] // Work around for rust-lang/rust#62127
pub use actix_macros::test;

/// The `TestServer` type.
///
/// `TestServer` is very simple test server that simplify process of writing
/// integration tests for actix-net applications.
///
/// # Examples
///
/// ```rust
/// use actix_service::fn_service;
/// use actix_testing::TestServer;
///
/// #[actix_rt::main]
/// async fn main() {
///     let srv = TestServer::with(|| fn_service(
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

/// Test server runtime
pub struct TestServerRuntime {
    addr: net::SocketAddr,
    host: String,
    port: u16,
    system: System,
}

impl TestServer {
    /// Start new server with server builder
    pub fn start<F>(mut factory: F) -> TestServerRuntime
    where
        F: FnMut(ServerBuilder) -> ServerBuilder + Send + 'static,
    {
        let (tx, rx) = mpsc::channel();

        // run server in separate thread
        thread::spawn(move || {
            let sys = System::new("actix-test-server");
            factory(Server::build())
                .workers(1)
                .disable_signals()
                .start();

            tx.send(System::current()).unwrap();
            sys.run()
        });
        let system = rx.recv().unwrap();

        TestServerRuntime {
            system,
            addr: "127.0.0.1:0".parse().unwrap(),
            host: "127.0.0.1".to_string(),
            port: 0,
        }
    }

    /// Start new test server with application factory
    pub fn with<F: ServiceFactory<TcpStream>>(factory: F) -> TestServerRuntime {
        let (tx, rx) = mpsc::channel();

        // run server in separate thread
        thread::spawn(move || {
            let sys = System::new("actix-test-server");
            let tcp = net::TcpListener::bind("127.0.0.1:0").unwrap();
            let local_addr = tcp.local_addr().unwrap();

            Server::build()
                .listen("test", tcp, factory)?
                .workers(1)
                .disable_signals()
                .start();

            tx.send((System::current(), local_addr)).unwrap();
            sys.run()
        });

        let (system, addr) = rx.recv().unwrap();

        let host = format!("{}", addr.ip());
        let port = addr.port();

        TestServerRuntime {
            system,
            addr,
            host,
            port,
        }
    }

    /// Get first available unused local address
    pub fn unused_addr() -> net::SocketAddr {
        let addr: net::SocketAddr = "127.0.0.1:0".parse().unwrap();
        let socket =
            Socket::new(Domain::ipv4(), Type::stream(), Some(Protocol::tcp())).unwrap();
        socket.bind(&addr.into()).unwrap();
        socket.set_reuse_address(true).unwrap();
        let tcp = socket.into_tcp_listener();
        tcp.local_addr().unwrap()
    }
}

impl TestServerRuntime {
    /// Test server host
    pub fn host(&self) -> &str {
        &self.host
    }

    /// Test server port
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Get test server address
    pub fn addr(&self) -> net::SocketAddr {
        self.addr
    }

    /// Stop http server
    fn stop(&mut self) {
        self.system.stop();
    }

    /// Connect to server, return tokio TcpStream
    pub fn connect(&self) -> std::io::Result<TcpStream> {
        TcpStream::from_std(net::TcpStream::connect(self.addr)?)
    }
}

impl Drop for TestServerRuntime {
    fn drop(&mut self) {
        self.stop()
    }
}
