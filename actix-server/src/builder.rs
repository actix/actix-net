use std::io;
use std::time::Duration;

use actix_rt::net::TcpStream;
use log::info;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

use crate::config::{ConfiguredService, ServiceConfig};
use crate::server::{Server, ServerCommand};
use crate::service::{InternalServiceFactory, ServiceFactory, StreamNewService};
use crate::socket::{
    MioListener, MioTcpListener, MioTcpSocket, StdSocketAddr, StdTcpListener, ToSocketAddrs,
};
use crate::worker::ServerWorkerConfig;
use crate::Token;

/// Server builder
pub struct ServerBuilder {
    pub(super) threads: usize,
    token: Token,
    backlog: u32,
    pub(super) services: Vec<Box<dyn InternalServiceFactory>>,
    pub(super) sockets: Vec<(Token, String, MioListener)>,
    pub(super) exit: bool,
    pub(super) no_signals: bool,
    pub(super) cmd_tx: UnboundedSender<ServerCommand>,
    pub(super) cmd_rx: UnboundedReceiver<ServerCommand>,
    pub(super) worker_config: ServerWorkerConfig,
}

impl Default for ServerBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ServerBuilder {
    /// Create new Server builder instance
    pub fn new() -> ServerBuilder {
        let (tx, rx) = unbounded_channel();
        ServerBuilder {
            threads: num_cpus::get(),
            token: Token::default(),
            services: Vec::new(),
            sockets: Vec::new(),
            backlog: 2048,
            exit: false,
            no_signals: false,
            cmd_tx: tx,
            cmd_rx: rx,
            worker_config: ServerWorkerConfig::default(),
        }
    }

    /// Set number of workers to start.
    ///
    /// By default server uses number of available logical cpu as workers
    /// count. Workers must be greater than 0.
    pub fn workers(mut self, num: usize) -> Self {
        assert_ne!(num, 0, "workers must be greater than 0");
        self.threads = num;
        self
    }

    /// Set max number of threads for each worker's blocking task thread pool.
    ///
    /// One thread pool is set up **per worker**; not shared across workers.
    ///
    /// # Examples:
    /// ```
    /// # use actix_server::ServerBuilder;
    /// let builder = ServerBuilder::new()
    ///     .workers(4) // server has 4 worker thread.
    ///     .worker_max_blocking_threads(4); // every worker has 4 max blocking threads.
    /// ```
    ///
    /// See [tokio::runtime::Builder::max_blocking_threads] for behavior reference.
    pub fn worker_max_blocking_threads(mut self, num: usize) -> Self {
        self.worker_config.max_blocking_threads(num);
        self
    }

    /// Set the maximum number of pending connections.
    ///
    /// This refers to the number of clients that can be waiting to be served.
    /// Exceeding this number results in the client getting an error when
    /// attempting to connect. It should only affect servers under significant
    /// load.
    ///
    /// Generally set in the 64-2048 range. Default value is 2048.
    ///
    /// This method should be called before `bind()` method call.
    pub fn backlog(mut self, num: u32) -> Self {
        self.backlog = num;
        self
    }

    /// Sets the maximum per-worker number of concurrent connections.
    ///
    /// All socket listeners will stop accepting connections when this limit is
    /// reached for each worker.
    ///
    /// By default max connections is set to a 25k per worker.
    pub fn maxconn(mut self, num: usize) -> Self {
        self.worker_config.max_concurrent_connections(num);
        self
    }

    /// Stop Actix system.
    pub fn system_exit(mut self) -> Self {
        self.exit = true;
        self
    }

    /// Disable signal handling.
    pub fn disable_signals(mut self) -> Self {
        self.no_signals = true;
        self
    }

    /// Timeout for graceful workers shutdown in seconds.
    ///
    /// After receiving a stop signal, workers have this much time to finish serving requests.
    /// Workers still alive after the timeout are force dropped.
    ///
    /// By default shutdown timeout sets to 30 seconds.
    pub fn shutdown_timeout(mut self, sec: u64) -> Self {
        self.worker_config
            .shutdown_timeout(Duration::from_secs(sec));
        self
    }

    /// Execute external configuration as part of the server building process.
    ///
    /// This function is useful for moving parts of configuration to a different module or
    /// even library.
    pub fn configure<F>(mut self, f: F) -> io::Result<ServerBuilder>
    where
        F: Fn(&mut ServiceConfig) -> io::Result<()>,
    {
        let mut cfg = ServiceConfig::new(self.threads, self.backlog);

        f(&mut cfg)?;

        if let Some(apply) = cfg.apply {
            let mut srv = ConfiguredService::new(apply);
            for (name, lst) in cfg.services {
                let token = self.token.next();
                srv.stream(token, name.clone(), lst.local_addr()?);
                self.sockets.push((token, name, MioListener::Tcp(lst)));
            }
            self.services.push(Box::new(srv));
        }
        self.threads = cfg.threads;

        Ok(self)
    }

    /// Add new service to the server.
    pub fn bind<F, U, N: AsRef<str>>(mut self, name: N, addr: U, factory: F) -> io::Result<Self>
    where
        F: ServiceFactory<TcpStream>,
        U: ToSocketAddrs,
    {
        let sockets = bind_addr(addr, self.backlog)?;

        for lst in sockets {
            let token = self.token.next();
            self.services.push(StreamNewService::create(
                name.as_ref().to_string(),
                token,
                factory.clone(),
                lst.local_addr()?,
            ));
            self.sockets
                .push((token, name.as_ref().to_string(), MioListener::Tcp(lst)));
        }
        Ok(self)
    }

    /// Add new unix domain service to the server.
    #[cfg(unix)]
    pub fn bind_uds<F, U, N>(self, name: N, addr: U, factory: F) -> io::Result<Self>
    where
        F: ServiceFactory<actix_rt::net::UnixStream>,
        N: AsRef<str>,
        U: AsRef<std::path::Path>,
    {
        // The path must not exist when we try to bind.
        // Try to remove it to avoid bind error.
        if let Err(e) = std::fs::remove_file(addr.as_ref()) {
            // NotFound is expected and not an issue. Anything else is.
            if e.kind() != std::io::ErrorKind::NotFound {
                return Err(e);
            }
        }

        let lst = crate::socket::StdUnixListener::bind(addr)?;
        self.listen_uds(name, lst, factory)
    }

    /// Add new unix domain service to the server.
    /// Useful when running as a systemd service and
    /// a socket FD can be acquired using the systemd crate.
    #[cfg(unix)]
    pub fn listen_uds<F, N: AsRef<str>>(
        mut self,
        name: N,
        lst: crate::socket::StdUnixListener,
        factory: F,
    ) -> io::Result<Self>
    where
        F: ServiceFactory<actix_rt::net::UnixStream>,
    {
        use std::net::{IpAddr, Ipv4Addr};
        lst.set_nonblocking(true)?;
        let token = self.token.next();
        let addr = StdSocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8080);
        self.services.push(StreamNewService::create(
            name.as_ref().to_string(),
            token,
            factory,
            addr,
        ));
        self.sockets
            .push((token, name.as_ref().to_string(), MioListener::from(lst)));
        Ok(self)
    }

    /// Add new service to the server.
    pub fn listen<F, N: AsRef<str>>(
        mut self,
        name: N,
        lst: StdTcpListener,
        factory: F,
    ) -> io::Result<Self>
    where
        F: ServiceFactory<TcpStream>,
    {
        lst.set_nonblocking(true)?;
        let addr = lst.local_addr()?;

        let token = self.token.next();
        self.services.push(StreamNewService::create(
            name.as_ref().to_string(),
            token,
            factory,
            addr,
        ));

        self.sockets
            .push((token, name.as_ref().to_string(), MioListener::from(lst)));

        Ok(self)
    }

    /// Starts processing incoming connections and return server controller.
    pub fn run(self) -> Server {
        if self.sockets.is_empty() {
            panic!("Server should have at least one bound socket");
        } else {
            info!("Starting {} workers", self.threads);
            Server::new(self)
        }
    }
}

pub(super) fn bind_addr<S: ToSocketAddrs>(
    addr: S,
    backlog: u32,
) -> io::Result<Vec<MioTcpListener>> {
    let mut err = None;
    let mut succ = false;
    let mut sockets = Vec::new();
    for addr in addr.to_socket_addrs()? {
        match create_tcp_listener(addr, backlog) {
            Ok(lst) => {
                succ = true;
                sockets.push(lst);
            }
            Err(e) => err = Some(e),
        }
    }

    if !succ {
        if let Some(e) = err.take() {
            Err(e)
        } else {
            Err(io::Error::new(
                io::ErrorKind::Other,
                "Can not bind to address.",
            ))
        }
    } else {
        Ok(sockets)
    }
}

fn create_tcp_listener(addr: StdSocketAddr, backlog: u32) -> io::Result<MioTcpListener> {
    let socket = match addr {
        StdSocketAddr::V4(_) => MioTcpSocket::new_v4()?,
        StdSocketAddr::V6(_) => MioTcpSocket::new_v6()?,
    };

    socket.set_reuseaddr(true)?;
    socket.bind(addr)?;
    socket.listen(backlog)
}
