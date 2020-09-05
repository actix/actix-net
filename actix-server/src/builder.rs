use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;
use std::{io, mem, net};

use actix_rt::net::TcpStream;
use actix_rt::time::{delay_until, Instant};
use actix_rt::{spawn, System};
use futures_channel::mpsc::{unbounded, UnboundedReceiver};
use futures_channel::oneshot;
use futures_util::future::ready;
use futures_util::stream::FuturesUnordered;
use futures_util::{future::Future, ready, stream::Stream, FutureExt, StreamExt};
use log::{error, info};
use socket2::{Domain, Protocol, Socket, Type};

use crate::accept::{AcceptLoop, AcceptNotify, Command};
use crate::config::{ConfiguredService, ServiceConfig};
use crate::server::{Server, ServerCommand};
use crate::service::{InternalServiceFactory, ServiceFactory, StreamNewService};
use crate::signals::{Signal, Signals};
use crate::socket::StdListener;
use crate::worker::{self, Worker, WorkerAvailability, WorkerClient};
use crate::Token;

/// Server builder
pub struct ServerBuilder {
    threads: usize,
    token: Token,
    backlog: i32,
    workers: Vec<(usize, WorkerClient)>,
    services: Vec<Box<dyn InternalServiceFactory>>,
    sockets: Vec<(Token, String, StdListener)>,
    accept: AcceptLoop,
    exit: bool,
    shutdown_timeout: Duration,
    no_signals: bool,
    cmd: UnboundedReceiver<ServerCommand>,
    server: Server,
    notify: Vec<oneshot::Sender<()>>,
}

impl Default for ServerBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ServerBuilder {
    /// Create new Server builder instance
    pub fn new() -> ServerBuilder {
        let (tx, rx) = unbounded();
        let server = Server::new(tx);

        ServerBuilder {
            threads: num_cpus::get(),
            token: Token(0),
            workers: Vec::new(),
            services: Vec::new(),
            sockets: Vec::new(),
            accept: AcceptLoop::new(server.clone()),
            backlog: 2048,
            exit: false,
            shutdown_timeout: Duration::from_secs(30),
            no_signals: false,
            cmd: rx,
            notify: Vec::new(),
            server,
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
    pub fn backlog(mut self, num: i32) -> Self {
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
        self.shutdown_timeout = Duration::from_secs(sec);
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
                self.sockets.push((token, name, StdListener::Tcp(lst)));
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
        U: net::ToSocketAddrs,
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
                .push((token, name.as_ref().to_string(), StdListener::Tcp(lst)));
        }
        Ok(self)
    }

    #[cfg(all(unix))]
    /// Add new unix domain service to the server.
    pub fn bind_uds<F, U, N>(self, name: N, addr: U, factory: F) -> io::Result<Self>
    where
        F: ServiceFactory<actix_rt::net::UnixStream>,
        N: AsRef<str>,
        U: AsRef<std::path::Path>,
    {
        use std::os::unix::net::UnixListener;

        // The path must not exist when we try to bind.
        // Try to remove it to avoid bind error.
        if let Err(e) = std::fs::remove_file(addr.as_ref()) {
            // NotFound is expected and not an issue. Anything else is.
            if e.kind() != std::io::ErrorKind::NotFound {
                return Err(e);
            }
        }

        let lst = UnixListener::bind(addr)?;
        self.listen_uds(name, lst, factory)
    }

    #[cfg(all(unix))]
    /// Add new unix domain service to the server.
    /// Useful when running as a systemd service and
    /// a socket FD can be acquired using the systemd crate.
    pub fn listen_uds<F, N: AsRef<str>>(
        mut self,
        name: N,
        lst: std::os::unix::net::UnixListener,
        factory: F,
    ) -> io::Result<Self>
    where
        F: ServiceFactory<actix_rt::net::UnixStream>,
    {
        use std::net::{IpAddr, Ipv4Addr, SocketAddr};
        let token = self.token.next();
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8080);
        self.services.push(StreamNewService::create(
            name.as_ref().to_string(),
            token,
            factory,
            addr,
        ));
        self.sockets
            .push((token, name.as_ref().to_string(), StdListener::Uds(lst)));
        Ok(self)
    }

    /// Add new service to the server.
    pub fn listen<F, N: AsRef<str>>(
        mut self,
        name: N,
        lst: net::TcpListener,
        factory: F,
    ) -> io::Result<Self>
    where
        F: ServiceFactory<TcpStream>,
    {
        let token = self.token.next();
        self.services.push(StreamNewService::create(
            name.as_ref().to_string(),
            token,
            factory,
            lst.local_addr()?,
        ));
        self.sockets
            .push((token, name.as_ref().to_string(), StdListener::Tcp(lst)));
        Ok(self)
    }

    #[doc(hidden)]
    pub fn start(self) -> Server {
        self.run()
    }

    /// Starts processing incoming connections and return server controller.
    pub fn run(mut self) -> Server {
        if self.sockets.is_empty() {
            panic!("Server should have at least one bound socket");
        } else {
            info!("Starting {} workers", self.threads);

            // start workers
            let workers = (0..self.threads)
                .map(|idx| {
                    let worker = self.start_worker(idx, self.accept.get_notify());
                    self.workers.push((idx, worker.clone()));

                    worker
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
                workers,
            );

            // handle signals
            if !self.no_signals {
                Signals::start(self.server.clone()).unwrap();
            }

            // start http server actor
            let server = self.server.clone();
            spawn(self);
            server
        }
    }

    fn start_worker(&self, idx: usize, notify: AcceptNotify) -> WorkerClient {
        let avail = WorkerAvailability::new(notify);
        let services: Vec<Box<dyn InternalServiceFactory>> =
            self.services.iter().map(|v| v.clone_factory()).collect();

        Worker::start(idx, services, avail, self.shutdown_timeout)
    }

    fn handle_cmd(&mut self, item: ServerCommand) {
        match item {
            ServerCommand::Pause(tx) => {
                self.accept.send(Command::Pause);
                let _ = tx.send(());
            }
            ServerCommand::Resume(tx) => {
                self.accept.send(Command::Resume);
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
                self.accept.send(Command::Stop);
                let notify = std::mem::take(&mut self.notify);

                // stop workers
                if !self.workers.is_empty() && graceful {
                    spawn(
                        self.workers
                            .iter()
                            .map(move |worker| worker.1.stop(graceful))
                            .collect::<FuturesUnordered<_>>()
                            .collect::<Vec<_>>()
                            .then(move |_| {
                                if let Some(tx) = completion {
                                    let _ = tx.send(());
                                }
                                for tx in notify {
                                    let _ = tx.send(());
                                }
                                if exit {
                                    spawn(
                                        async {
                                            delay_until(
                                                Instant::now() + Duration::from_millis(300),
                                            )
                                            .await;
                                            System::current().stop();
                                        }
                                        .boxed(),
                                    );
                                }
                                ready(())
                            }),
                    )
                } else {
                    // we need to stop system if server was spawned
                    if self.exit {
                        spawn(
                            delay_until(Instant::now() + Duration::from_millis(300)).then(
                                |_| {
                                    System::current().stop();
                                    ready(())
                                },
                            ),
                        );
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
                for i in 0..self.workers.len() {
                    if self.workers[i].0 == idx {
                        self.workers.swap_remove(i);
                        found = true;
                        break;
                    }
                }

                if found {
                    error!("Worker has died {:?}, restarting", idx);

                    let mut new_idx = self.workers.len();
                    'found: loop {
                        for i in 0..self.workers.len() {
                            if self.workers[i].0 == new_idx {
                                new_idx += 1;
                                continue 'found;
                            }
                        }
                        break;
                    }

                    let worker = self.start_worker(new_idx, self.accept.get_notify());
                    self.workers.push((new_idx, worker.clone()));
                    self.accept.send(Command::Worker(worker));
                }
            }
        }
    }
}

impl Future for ServerBuilder {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        loop {
            match ready!(Pin::new(&mut self.cmd).poll_next(cx)) {
                Some(it) => self.as_mut().get_mut().handle_cmd(it),
                None => {
                    return Poll::Pending;
                }
            }
        }
    }
}

pub(super) fn bind_addr<S: net::ToSocketAddrs>(
    addr: S,
    backlog: i32,
) -> io::Result<Vec<net::TcpListener>> {
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

fn create_tcp_listener(addr: net::SocketAddr, backlog: i32) -> io::Result<net::TcpListener> {
    let domain = match addr {
        net::SocketAddr::V4(_) => Domain::ipv4(),
        net::SocketAddr::V6(_) => Domain::ipv6(),
    };
    let socket = Socket::new(domain, Type::stream(), Some(Protocol::tcp()))?;
    socket.set_reuse_address(true)?;
    socket.bind(&addr.into())?;
    socket.listen(backlog)?;
    Ok(socket.into_tcp_listener())
}
