use std::{
    fmt,
    future::Future,
    io, mem,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use actix_rt::{self as rt, net::TcpStream, time::sleep, System};
use actix_service::ServiceFactory;
use log::{error, info};
use tokio::sync::{
    mpsc::{unbounded_channel, UnboundedReceiver},
    oneshot,
};

use crate::{
    accept::AcceptLoop,
    join_all,
    server::{ServerCommand, ServerHandle},
    service::{ServerServiceFactory, StreamNewService},
    signals::{Signal, Signals},
    socket::{
        MioListener, MioTcpListener, MioTcpSocket, StdSocketAddr, StdTcpListener, ToSocketAddrs,
    },
    waker_queue::{WakerInterest, WakerQueue},
    worker::{ServerWorker, ServerWorkerConfig, WorkerHandleAccept, WorkerHandleServer},
};

/// Server builder
pub struct ServerBuilder {
    threads: usize,
    token: usize,
    backlog: u32,
    handles: Vec<(usize, WorkerHandleServer)>,
    services: Vec<Box<dyn ServerServiceFactory>>,
    sockets: Vec<(usize, String, MioListener)>,
    accept: AcceptLoop,
    exit: bool,
    no_signals: bool,
    cmd: UnboundedReceiver<ServerCommand>,
    server: ServerHandle,
    notify: Vec<oneshot::Sender<()>>,
    worker_config: ServerWorkerConfig,
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
        let server = ServerHandle::new(tx);

        ServerBuilder {
            threads: num_cpus::get(),
            token: 0,
            handles: Vec::new(),
            services: Vec::new(),
            sockets: Vec::new(),
            accept: AcceptLoop::new(server.clone()),
            backlog: 2048,
            exit: false,
            no_signals: false,
            cmd: rx,
            notify: Vec::new(),
            server,
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
    /// All socket listeners will stop accepting connections when this limit is reached for
    /// each worker.
    ///
    /// By default max connections is set to a 25,600 per worker.
    pub fn max_concurrent_connections(mut self, num: usize) -> Self {
        self.worker_config.max_concurrent_connections(num);
        self
    }

    #[doc(hidden)]
    #[deprecated(since = "2.0.0", note = "Renamed to `max_concurrent_connections`.")]
    pub fn maxconn(self, num: usize) -> Self {
        self.max_concurrent_connections(num)
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

    /// Bind server to socket addresses.
    ///
    /// Binds to all network interface addresses that resolve from the `addr` argument.
    /// Eg. using `localhost` might bind to both IPv4 and IPv6 addresses. Bind to multiple distinct
    /// interfaces at the same time by passing a list of socket addresses.
    pub fn bind<F, U, InitErr>(
        mut self,
        name: impl AsRef<str>,
        addr: U,
        factory: F,
    ) -> io::Result<Self>
    where
        F: ServiceFactory<TcpStream, Config = (), InitError = InitErr> + Send + Clone + 'static,
        InitErr: fmt::Debug + Send + 'static,
        U: ToSocketAddrs,
    {
        let sockets = bind_addr(addr, self.backlog)?;

        for lst in sockets {
            let token = self.next_token();

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

    /// Bind server to existing TCP listener.
    ///
    /// Useful when running as a systemd service and a socket FD can be passed to the process.
    pub fn listen<F, InitErr>(
        mut self,
        name: impl AsRef<str>,
        lst: StdTcpListener,
        factory: F,
    ) -> io::Result<Self>
    where
        F: ServiceFactory<TcpStream, Config = (), InitError = InitErr> + Send + Clone + 'static,
        InitErr: fmt::Debug + Send + 'static,
    {
        lst.set_nonblocking(true)?;

        let addr = lst.local_addr()?;
        let token = self.next_token();

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
    pub fn run(mut self) -> ServerHandle {
        if self.sockets.is_empty() {
            panic!("Server should have at least one bound socket");
        } else {
            for (_, name, lst) in &self.sockets {
                info!(
                    r#"Starting service: "{}", workers: {}, listening on: {}"#,
                    name,
                    self.threads,
                    lst.local_addr()
                );
            }

            // start workers
            let handles = (0..self.threads)
                .map(|idx| {
                    let (handle_accept, handle_server) =
                        self.start_worker(idx, self.accept.waker_owned());
                    self.handles.push((idx, handle_server));

                    handle_accept
                })
                .collect();

            // start accept thread
            self.accept.start(
                mem::take(&mut self.sockets)
                    .into_iter()
                    .map(|t| (t.0, t.2))
                    .collect(),
                handles,
            );

            // handle signals
            if !self.no_signals {
                Signals::start(self.server.clone());
            }

            // start http server
            let server = self.server.clone();
            rt::spawn(self);
            server
        }
    }

    fn start_worker(
        &self,
        idx: usize,
        waker_queue: WakerQueue,
    ) -> (WorkerHandleAccept, WorkerHandleServer) {
        let services = self.services.iter().map(|v| v.clone_factory()).collect();

        ServerWorker::start(idx, services, waker_queue, self.worker_config)
    }

    fn handle_cmd(&mut self, item: ServerCommand) {
        match item {
            ServerCommand::Pause(tx) => {
                self.accept.wake(WakerInterest::Pause);
                let _ = tx.send(());
            }
            ServerCommand::Resume(tx) => {
                self.accept.wake(WakerInterest::Resume);
                let _ = tx.send(());
            }
            ServerCommand::Signal(sig) => {
                // Signals support
                // Handle `SIGINT`, `SIGTERM`, `SIGQUIT` signals and stop actix system
                match sig {
                    Signal::Int => {
                        info!("SIGINT received; starting forced shutdown");
                        self.exit = true;
                        self.handle_cmd(ServerCommand::Stop {
                            graceful: false,
                            completion: None,
                        })
                    }

                    Signal::Term => {
                        info!("SIGTERM received; starting graceful shutdown");
                        self.exit = true;
                        self.handle_cmd(ServerCommand::Stop {
                            graceful: true,
                            completion: None,
                        })
                    }

                    Signal::Quit => {
                        info!("SIGQUIT received; starting forced shutdown");
                        self.exit = true;
                        self.handle_cmd(ServerCommand::Stop {
                            graceful: false,
                            completion: None,
                        })
                    }
                }
            }
            ServerCommand::Notify(tx) => {
                self.notify.push(tx);
            }
            ServerCommand::Stop {
                graceful,
                completion,
            } => {
                let exit = self.exit;

                // stop accept thread
                self.accept.wake(WakerInterest::Stop);
                let notify = std::mem::take(&mut self.notify);

                // stop workers
                let stop = self
                    .handles
                    .iter()
                    .map(move |worker| worker.1.stop(graceful))
                    .collect();

                rt::spawn(async move {
                    if graceful {
                        // wait for all workers to shut down
                        let _ = join_all(stop).await;
                    }

                    if let Some(tx) = completion {
                        let _ = tx.send(());
                    }

                    for tx in notify {
                        let _ = tx.send(());
                    }

                    if exit {
                        sleep(Duration::from_millis(300)).await;
                        System::current().stop();
                    }
                });
            }
            ServerCommand::WorkerFaulted(idx) => {
                let mut found = false;
                for i in 0..self.handles.len() {
                    if self.handles[i].0 == idx {
                        self.handles.swap_remove(i);
                        found = true;
                        break;
                    }
                }

                if found {
                    error!("Worker {} has died; restarting", idx);

                    let mut new_idx = self.handles.len();
                    'found: loop {
                        for i in 0..self.handles.len() {
                            if self.handles[i].0 == new_idx {
                                new_idx += 1;
                                continue 'found;
                            }
                        }
                        break;
                    }

                    let (handle_accept, handle_server) =
                        self.start_worker(new_idx, self.accept.waker_owned());
                    self.handles.push((new_idx, handle_server));
                    self.accept.wake(WakerInterest::Worker(handle_accept));
                }
            }
        }
    }

    fn next_token(&mut self) -> usize {
        let token = self.token;
        self.token += 1;
        token
    }
}

/// Unix Domain Socket (UDS) support.
#[cfg(unix)]
impl ServerBuilder {
    /// Add new unix domain service to the server.
    pub fn bind_uds<F, U, InitErr>(
        self,
        name: impl AsRef<str>,
        addr: U,
        factory: F,
    ) -> io::Result<Self>
    where
        F: ServiceFactory<actix_rt::net::UnixStream, Config = (), InitError = InitErr>
            + Send
            + Clone
            + 'static,
        U: AsRef<std::path::Path>,
        InitErr: fmt::Debug + Send + 'static,
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
    ///
    /// Useful when running as a systemd service and a socket FD can be passed to the process.
    pub fn listen_uds<F, InitErr>(
        mut self,
        name: impl AsRef<str>,
        lst: crate::socket::StdUnixListener,
        factory: F,
    ) -> io::Result<Self>
    where
        F: ServiceFactory<actix_rt::net::UnixStream, Config = (), InitError = InitErr>
            + Send
            + Clone
            + 'static,
        InitErr: fmt::Debug + Send + 'static,
    {
        use std::net::{IpAddr, Ipv4Addr};

        lst.set_nonblocking(true)?;

        let addr = StdSocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8080);
        let token = self.next_token();

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
}

impl Future for ServerBuilder {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        loop {
            match Pin::new(&mut self.cmd).poll_recv(cx) {
                Poll::Ready(Some(it)) => self.as_mut().get_mut().handle_cmd(it),
                _ => return Poll::Pending,
            }
        }
    }
}

pub(super) fn bind_addr<S: ToSocketAddrs>(
    addr: S,
    backlog: u32,
) -> io::Result<Vec<MioTcpListener>> {
    let mut err = None;
    let mut success = false;
    let mut sockets = Vec::new();

    for addr in addr.to_socket_addrs()? {
        match create_tcp_listener(addr, backlog) {
            Ok(lst) => {
                success = true;
                sockets.push(lst);
            }
            Err(e) => err = Some(e),
        }
    }

    if success {
        Ok(sockets)
    } else if let Some(err) = err.take() {
        Err(err)
    } else {
        Err(io::Error::new(
            io::ErrorKind::Other,
            "Can not bind to socket address",
        ))
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
