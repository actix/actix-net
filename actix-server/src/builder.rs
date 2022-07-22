use std::{io, time::Duration};

use actix_rt::net::TcpStream;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tracing::{info, trace};

use crate::{
    server::ServerCommand,
    service::{InternalServiceFactory, ServerServiceFactory, StreamNewService},
    socket::{
        create_mio_tcp_listener, MioListener, MioTcpListener, StdTcpListener, ToSocketAddrs,
    },
    worker::ServerWorkerConfig,
    Server,
};

/// [Server] builder.
pub struct ServerBuilder {
    pub(crate) threads: usize,
    pub(crate) token: usize,
    pub(crate) backlog: u32,
    pub(crate) factories: Vec<Box<dyn InternalServiceFactory>>,
    pub(crate) sockets: Vec<(usize, String, MioListener)>,
    pub(crate) exit: bool,
    pub(crate) listen_os_signals: bool,
    pub(crate) cmd_tx: UnboundedSender<ServerCommand>,
    pub(crate) cmd_rx: UnboundedReceiver<ServerCommand>,
    pub(crate) worker_config: ServerWorkerConfig,
}

impl Default for ServerBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ServerBuilder {
    /// Create new Server builder instance
    pub fn new() -> ServerBuilder {
        let (cmd_tx, cmd_rx) = unbounded_channel();

        ServerBuilder {
            threads: num_cpus::get_physical(),
            token: 0,
            factories: Vec::new(),
            sockets: Vec::new(),
            backlog: 2048,
            exit: false,
            listen_os_signals: true,
            cmd_tx,
            cmd_rx,
            worker_config: ServerWorkerConfig::default(),
        }
    }

    /// Set number of workers to start.
    ///
    /// `num` must be greater than 0.
    ///
    /// The default worker count is the number of physical CPU cores available. If your benchmark
    /// testing indicates that simultaneous multi-threading is beneficial to your app, you can use
    /// the [`num_cpus`] crate to acquire the _logical_ core count instead.
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
    /// This refers to the number of clients that can be waiting to be served. Exceeding this number
    /// results in the client getting an error when attempting to connect. It should only affect
    /// servers under significant load.
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
    /// All socket listeners will stop accepting connections when this limit is reached for
    /// each worker.
    ///
    /// By default max connections is set to a 25k per worker.
    pub fn max_concurrent_connections(mut self, num: usize) -> Self {
        self.worker_config.max_concurrent_connections(num);
        self
    }

    #[doc(hidden)]
    #[deprecated(since = "2.0.0", note = "Renamed to `max_concurrent_connections`.")]
    pub fn maxconn(self, num: usize) -> Self {
        self.max_concurrent_connections(num)
    }

    /// Stop Actix `System` after server shutdown.
    pub fn system_exit(mut self) -> Self {
        self.exit = true;
        self
    }

    /// Disable OS signal handling.
    pub fn disable_signals(mut self) -> Self {
        self.listen_os_signals = false;
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

    /// Add new service to the server.
    pub fn bind<F, U, N>(mut self, name: N, addr: U, factory: F) -> io::Result<Self>
    where
        F: ServerServiceFactory<TcpStream>,
        U: ToSocketAddrs,
        N: AsRef<str>,
    {
        let sockets = bind_addr(addr, self.backlog)?;

        trace!("binding server to: {:?}", &sockets);

        for lst in sockets {
            let token = self.next_token();
            self.factories.push(StreamNewService::create(
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

    /// Add new service to the server.
    pub fn listen<F, N: AsRef<str>>(
        mut self,
        name: N,
        lst: StdTcpListener,
        factory: F,
    ) -> io::Result<Self>
    where
        F: ServerServiceFactory<TcpStream>,
    {
        lst.set_nonblocking(true)?;
        let addr = lst.local_addr()?;

        let token = self.next_token();
        self.factories.push(StreamNewService::create(
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
            info!("starting {} workers", self.threads);
            Server::new(self)
        }
    }

    fn next_token(&mut self) -> usize {
        let token = self.token;
        self.token += 1;
        token
    }
}

#[cfg(unix)]
impl ServerBuilder {
    /// Add new unix domain service to the server.
    pub fn bind_uds<F, U, N>(self, name: N, addr: U, factory: F) -> io::Result<Self>
    where
        F: ServerServiceFactory<actix_rt::net::UnixStream>,
        N: AsRef<str>,
        U: AsRef<std::path::Path>,
    {
        // The path must not exist when we try to bind.
        // Try to remove it to avoid bind error.
        if let Err(err) = std::fs::remove_file(addr.as_ref()) {
            // NotFound is expected and not an issue. Anything else is.
            if err.kind() != std::io::ErrorKind::NotFound {
                return Err(err);
            }
        }

        let lst = crate::socket::StdUnixListener::bind(addr)?;
        self.listen_uds(name, lst, factory)
    }

    /// Add new unix domain service to the server.
    ///
    /// Useful when running as a systemd service and a socket FD is acquired externally.
    pub fn listen_uds<F, N: AsRef<str>>(
        mut self,
        name: N,
        lst: crate::socket::StdUnixListener,
        factory: F,
    ) -> io::Result<Self>
    where
        F: ServerServiceFactory<actix_rt::net::UnixStream>,
    {
        use std::net::{IpAddr, Ipv4Addr};
        lst.set_nonblocking(true)?;
        let token = self.next_token();
        let addr =
            crate::socket::StdSocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8080);
        self.factories.push(StreamNewService::create(
            name.as_ref().to_string(),
            token,
            factory,
            addr,
        ));
        self.sockets
            .push((token, name.as_ref().to_string(), MioListener::from(lst)));
        Ok(self)
    }
}

pub(super) fn bind_addr<S: ToSocketAddrs>(
    addr: S,
    backlog: u32,
) -> io::Result<Vec<MioTcpListener>> {
    let mut opt_err = None;
    let mut success = false;
    let mut sockets = Vec::new();

    for addr in addr.to_socket_addrs()? {
        match create_mio_tcp_listener(addr, backlog) {
            Ok(lst) => {
                success = true;
                sockets.push(lst);
            }
            Err(err) => opt_err = Some(err),
        }
    }

    if success {
        Ok(sockets)
    } else if let Some(err) = opt_err.take() {
        Err(err)
    } else {
        Err(io::Error::new(
            io::ErrorKind::Other,
            "Can not bind to address.",
        ))
    }
}
