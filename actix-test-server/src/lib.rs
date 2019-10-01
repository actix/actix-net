//! Various helpers for Actix applications to use during testing.
use std::sync::mpsc;
use std::{net, thread};

use actix_rt::{Runtime, System};
use actix_server::{Server, StreamServiceFactory};
pub use actix_server_config::{Io, ServerConfig};

use futures::future::{lazy, Future, IntoFuture};
use net2::TcpBuilder;
use tokio_reactor::Handle;
use tokio_tcp::TcpStream;

/// The `TestServer` type.
///
/// `TestServer` is very simple test server that simplify process of writing
/// integration tests for actix-net applications.
///
/// # Examples
///
/// ```rust
/// use actix_service::{service_fn, IntoNewService};
/// use actix_test_server::TestServer;
///
/// fn main() {
///     let srv = TestServer::with(|| service_fn(
///         |sock| {
///             println!("New connection: {:?}", sock);
///             Ok::<_, ()>(())
///         }
///     ));
///
///     println!("SOCKET: {:?}", srv.connect());
/// }
/// ```
pub struct TestServer;

/// Test server runstime
pub struct TestServerRuntime {
    addr: net::SocketAddr,
    host: String,
    port: u16,
    rt: Runtime,
}

impl TestServer {
    /// Start new test server with application factory
    pub fn with<F: StreamServiceFactory>(factory: F) -> TestServerRuntime {
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
        System::set_current(system);

        let rt = Runtime::new().unwrap();
        let host = format!("{}", addr.ip());
        let port = addr.port();

        TestServerRuntime {
            addr,
            rt,
            host,
            port,
        }
    }

    /// Get firat available unused local address
    pub fn unused_addr() -> net::SocketAddr {
        let addr: net::SocketAddr = "127.0.0.1:0".parse().unwrap();
        let socket = TcpBuilder::new_v4().unwrap();
        socket.bind(&addr).unwrap();
        socket.reuse_address(true).unwrap();
        let tcp = socket.to_tcp_listener().unwrap();
        tcp.local_addr().unwrap()
    }
}

impl TestServerRuntime {
    /// Execute future on current runtime
    pub fn block_on<F, I, E>(&mut self, fut: F) -> Result<I, E>
    where
        F: Future<Item = I, Error = E>,
    {
        self.rt.block_on(fut)
    }

    /// Runs the provided function, with runtime enabled.
    pub fn run_on<F, R>(&mut self, f: F) -> Result<R::Item, R::Error>
    where
        F: FnOnce() -> R,
        R: IntoFuture,
    {
        self.rt.block_on(lazy(|| f().into_future()))
    }

    /// Spawn future to the current runtime
    pub fn spawn<F>(&mut self, fut: F)
    where
        F: Future<Item = (), Error = ()> + 'static,
    {
        self.rt.spawn(fut);
    }

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
        System::current().stop();
    }

    /// Connect to server, return tokio TcpStream
    pub fn connect(&self) -> std::io::Result<TcpStream> {
        TcpStream::from_std(net::TcpStream::connect(self.addr)?, &Handle::default())
    }
}

impl Drop for TestServerRuntime {
    fn drop(&mut self) {
        self.stop()
    }
}
