use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;
use std::{io, mem};

use actix_rt::net::TcpStream;
use actix_rt::time::{sleep_until, Instant};
use actix_rt::{self as rt, System};
use log::{error, info};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};
use tokio::sync::oneshot;

use crate::accept::AcceptLoop;
use crate::config::{ConfiguredService, ServiceConfig};
use crate::server::{Server, ServerCommand};
use crate::service::{InternalServiceFactory, ServiceFactory, StreamNewService};
use crate::signals::{Signal, Signals};
use crate::socket::{MioListener, StdSocketAddr, StdTcpListener, ToSocketAddrs};
use crate::socket::{MioTcpListener, MioTcpSocket};
use crate::waker_queue::{WakerInterest, WakerQueue};
use crate::worker::{self, ServerWorker, ServerWorkerConfig, WorkerAvailability, WorkerHandle};
use crate::{join_all, Token};

/// Server builder
pub struct ServerBuilder {
    threads: usize,
    token: Token,
    backlog: u32,
    handles: Vec<(usize, WorkerHandle)>,
    services: Vec<Box<dyn InternalServiceFactory>>,
    sockets: Vec<(Token, String, MioListener)>,
    accept: AcceptLoop,
    exit: bool,
    no_signals: bool,
    cmd: UnboundedReceiver<ServerCommand>,
    server: Server,
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
        let server = Server::new(tx);

        ServerBuilder {
            threads: num_cpus::get(),
            token: Token::default(),
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
    pub fn run(mut self) -> Server {
        if self.sockets.is_empty() {
            panic!("Server should have at least one bound socket");
        } else {
            info!("Starting {} workers", self.threads);

            // start workers
            let handles = (0..self.threads)
                .map(|idx| {
                    let handle = self.start_worker(idx, self.accept.waker_owned());
                    self.handles.push((idx, handle.clone()));

                    handle
                })
                .collect();

            // start accept thread
            for sock in &self.sockets {
                info!("Starting \"{}\" service on {}", sock.1, sock.2);
            }
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

            // start http server actor
            let server = self.server.clone();
            rt::spawn(self);
            server
        }
    }

    fn start_worker(&self, idx: usize, waker: WakerQueue) -> WorkerHandle {
        let avail = WorkerAvailability::new(waker);
        let services = self.services.iter().map(|v| v.clone_factory()).collect();

        ServerWorker::start(idx, services, avail, self.worker_config)
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
                    _ => (),
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
                if !self.handles.is_empty() && graceful {
                    let iter = self
                        .handles
                        .iter()
                        .map(move |worker| worker.1.stop(graceful))
                        .collect();

                    let fut = join_all(iter);

                    rt::spawn(async move {
                        let _ = fut.await;
                        if let Some(tx) = completion {
                            let _ = tx.send(());
                        }
                        for tx in notify {
                            let _ = tx.send(());
                        }
                        if exit {
                            rt::spawn(async {
                                sleep_until(Instant::now() + Duration::from_millis(300)).await;
                                System::current().stop();
                            });
                        }
                    });
                } else {
                    // we need to stop system if server was spawned
                    if self.exit {
                        rt::spawn(async {
                            sleep_until(Instant::now() + Duration::from_millis(300)).await;
                            System::current().stop();
                        });
                    }
                    if let Some(tx) = completion {
                        let _ = tx.send(());
                    }
                    for tx in notify {
                        let _ = tx.send(());
                    }
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

                    let handle = self.start_worker(new_idx, self.accept.waker_owned());
                    self.handles.push((new_idx, handle.clone()));
                    self.accept.wake(WakerInterest::Worker(handle));
                }
            }
        }
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
