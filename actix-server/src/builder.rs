use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;
use std::{io, mem};

use actix_rt::net::TcpStream;
use actix_rt::time::{sleep_until, Instant};
use actix_rt::System;
use futures_core::future::BoxFuture;
use log::{error, info};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::sync::oneshot;

use crate::accept::Accept;
use crate::config::{ConfiguredService, ServiceConfig};
use crate::server_handle::{ServerCommand, ServerHandle};
use crate::service::{InternalServiceFactory, ServiceFactory, StreamNewService};
use crate::signals::{Signal, Signals};
use crate::socket::{MioListener, StdSocketAddr, StdTcpListener, ToSocketAddrs};
use crate::socket::{MioTcpListener, MioTcpSocket};
use crate::waker_queue::{WakerInterest, WakerQueue};
use crate::worker::{self, ServerWorker, ServerWorkerConfig, WorkerAvailability, WorkerHandle};
use crate::Token;

/// Server builder
pub struct ServerBuilder {
    threads: usize,
    token: Token,
    backlog: u32,
    services: Vec<Box<dyn InternalServiceFactory>>,
    sockets: Vec<(Token, String, MioListener)>,
    exit: bool,
    no_signals: bool,
    cmd_tx: UnboundedSender<ServerCommand>,
    cmd_rx: UnboundedReceiver<ServerCommand>,
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
    pub fn maxconn(self, num: usize) -> Self {
        worker::max_concurrent_connections(num);
        self
    }

    /// Stop actix system.
    pub fn system_exit(mut self) -> Self {
        self.exit = true;
        self
    }

    /// Disable signal handling
    pub fn disable_signals(mut self) -> Self {
        self.no_signals = true;
        self
    }

    /// Timeout for graceful workers shutdown in seconds.
    ///
    /// After receiving a stop signal, workers have this much time to finish
    /// serving requests. Workers still alive after the timeout are force
    /// dropped.
    ///
    /// By default shutdown timeout sets to 30 seconds.
    pub fn shutdown_timeout(mut self, sec: u64) -> Self {
        self.worker_config
            .shutdown_timeout(Duration::from_secs(sec));
        self
    }

    /// Execute external configuration as part of the server building
    /// process.
    ///
    /// This function is useful for moving parts of configuration to a
    /// different module or even library.
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
    pub fn run(mut self) -> ServerFuture {
        if self.sockets.is_empty() {
            panic!("Server should have at least one bound socket");
        } else {
            info!("Starting {} workers", self.threads);

            let sockets = mem::take(&mut self.sockets)
                .into_iter()
                .map(|t| {
                    info!("Starting \"{}\" service on {}", t.1, t.2);
                    (t.0, t.2)
                })
                .collect();

            // collect worker handles on start.
            let mut handles = Vec::new();

            // start accept thread. return waker_queue for wake up it.
            let waker_queue = Accept::start(
                sockets,
                ServerHandle::new(self.cmd_tx.clone()),
                // closure for construct worker and return it's handler.
                |waker| {
                    (0..self.threads)
                        .map(|idx| {
                            // start workers
                            let availability = WorkerAvailability::new(waker.clone());
                            let factories =
                                self.services.iter().map(|v| v.clone_factory()).collect();
                            let handle = ServerWorker::start(
                                idx,
                                factories,
                                availability,
                                self.worker_config,
                            );
                            handles.push((idx, handle.clone()));
                            handle
                        })
                        .collect()
                },
            );

            // construct signals future.
            let signals = if !self.no_signals {
                Some(Signals::new())
            } else {
                None
            };

            ServerFuture {
                cmd_tx: self.cmd_tx,
                cmd_rx: self.cmd_rx,
                handles,
                services: self.services,
                notify: Vec::new(),
                exit: self.exit,
                worker_config: self.worker_config,
                signals,
                on_stop_task: None,
                waker_queue,
            }
        }
    }
}

/// When awaited or spawned would listen to signal and message from [ServerHandle](crate::server::ServerHandle).
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct ServerFuture {
    cmd_tx: UnboundedSender<ServerCommand>,
    cmd_rx: UnboundedReceiver<ServerCommand>,
    handles: Vec<(usize, WorkerHandle)>,
    services: Vec<Box<dyn InternalServiceFactory>>,
    notify: Vec<oneshot::Sender<()>>,
    exit: bool,
    worker_config: ServerWorkerConfig,
    signals: Option<Signals>,
    on_stop_task: Option<BoxFuture<'static, ()>>,
    waker_queue: WakerQueue,
}

impl ServerFuture {
    /// Obtain a Handle for ServerFuture that can be used to change state of actix server.
    ///
    /// See [ServerHandle](crate::server::ServerHandle) for usage.
    pub fn handle(&self) -> ServerHandle {
        ServerHandle::new(self.cmd_tx.clone())
    }

    fn handle_cmd(&mut self, item: ServerCommand) -> Option<BoxFuture<'static, ()>> {
        match item {
            ServerCommand::Pause(tx) => {
                self.waker_queue.wake(WakerInterest::Pause);
                let _ = tx.send(());
                None
            }
            ServerCommand::Resume(tx) => {
                self.waker_queue.wake(WakerInterest::Resume);
                let _ = tx.send(());
                None
            }
            ServerCommand::Signal(sig) => {
                // Signals support
                // Handle `SIGINT`, `SIGTERM`, `SIGQUIT` signals and stop actix system
                match sig {
                    Signal::Int => {
                        info!("SIGINT received, exiting");
                        self.exit = true;
                        self.handle_cmd(ServerCommand::Stop {
                            graceful: false,
                            completion: None,
                        })
                    }
                    Signal::Term => {
                        info!("SIGTERM received, stopping");
                        self.exit = true;
                        self.handle_cmd(ServerCommand::Stop {
                            graceful: true,
                            completion: None,
                        })
                    }
                    Signal::Quit => {
                        info!("SIGQUIT received, exiting");
                        self.exit = true;
                        self.handle_cmd(ServerCommand::Stop {
                            graceful: false,
                            completion: None,
                        })
                    }
                    _ => None,
                }
            }
            ServerCommand::Notify(tx) => {
                self.notify.push(tx);
                None
            }
            ServerCommand::Stop {
                graceful,
                completion,
            } => {
                let exit = self.exit;

                // stop accept thread
                self.waker_queue.wake(WakerInterest::Stop);
                let notify = std::mem::take(&mut self.notify);

                // stop workers
                if !self.handles.is_empty() && graceful {
                    let iter = self
                        .handles
                        .iter()
                        .map(move |worker| worker.1.stop(graceful))
                        .collect::<Vec<_>>();

                    // TODO: this async block can return io::Error.
                    Some(Box::pin(async move {
                        for handle in iter {
                            let _ = handle.await;
                        }
                        if let Some(tx) = completion {
                            let _ = tx.send(());
                        }
                        for tx in notify {
                            let _ = tx.send(());
                        }
                        if exit {
                            sleep_until(Instant::now() + Duration::from_millis(300)).await;
                            System::current().stop();
                        }
                    }))
                } else {
                    // we need to stop system if server was spawned
                    let exit = self.exit;
                    // TODO: this async block can return io::Error.
                    Some(Box::pin(async move {
                        if exit {
                            sleep_until(Instant::now() + Duration::from_millis(300)).await;
                            System::current().stop();
                        }
                        if let Some(tx) = completion {
                            let _ = tx.send(());
                        }
                        for tx in notify {
                            let _ = tx.send(());
                        }
                    }))
                }
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
                    error!("Worker has died {:?}, restarting", idx);

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

                    let availability = WorkerAvailability::new(self.waker_queue.clone());
                    let factories = self.services.iter().map(|v| v.clone_factory()).collect();
                    let handle = ServerWorker::start(
                        new_idx,
                        factories,
                        availability,
                        self.worker_config,
                    );

                    self.handles.push((new_idx, handle.clone()));
                    self.waker_queue.wake(WakerInterest::Worker(handle));
                }
                None
            }
        }
    }
}

impl Future for ServerFuture {
    type Output = io::Result<()>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.as_mut().get_mut();

        // poll signals first. remove it on resolve.
        if let Some(ref mut signals) = this.signals {
            if let Poll::Ready(signal) = Pin::new(signals).poll(cx) {
                this.on_stop_task = this.handle_cmd(ServerCommand::Signal(signal));
                this.signals = None;
            }
        }

        // actively poll command channel and handle command.
        loop {
            // got on stop task. resolve it exclusively and exit.
            if let Some(ref mut fut) = this.on_stop_task {
                return fut.as_mut().poll(cx).map(|_| Ok(()));
            }

            match Pin::new(&mut this.cmd_rx).poll_recv(cx) {
                Poll::Ready(Some(it)) => {
                    this.on_stop_task = this.handle_cmd(it);
                }
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
