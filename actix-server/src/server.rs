use std::{
    future::Future,
    io, mem,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use actix_rt::{time::sleep, System};
use futures_core::future::BoxFuture;
use log::{error, info};
use tokio::sync::{
    mpsc::{UnboundedReceiver, UnboundedSender},
    oneshot,
};

use crate::{
    accept::Accept,
    builder::ServerBuilder,
    join_all::join_all,
    service::InternalServiceFactory,
    signals::{Signal, Signals},
    waker_queue::{WakerInterest, WakerQueue},
    worker::{ServerWorker, ServerWorkerConfig, WorkerHandleServer},
};

#[derive(Debug)]
pub(crate) enum ServerCommand {
    /// TODO
    WorkerFaulted(usize),

    /// Contains return channel to notify caller of successful state change.
    Pause(oneshot::Sender<()>),

    /// Contains return channel to notify caller of successful state change.
    Resume(oneshot::Sender<()>),

    /// TODO
    Stop {
        /// True if shut down should be graceful.
        graceful: bool,

        /// Return channel to notify caller that shutdown is complete.
        completion: Option<oneshot::Sender<()>>,
    },
}

/// Server
///
/// # Shutdown Signals
/// On UNIX systems, `SIGQUIT` will start a graceful shutdown and `SIGTERM` or `SIGINT` will start a
/// forced shutdown. On Windows, a CTRL-C signal will start a forced shutdown.
///
/// A graceful shutdown will wait for all workers to stop first.
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub enum Server {
    Server(ServerInner),
    Error(Option<io::Error>),
}

impl Server {
    /// Start server building process.
    pub fn build() -> ServerBuilder {
        ServerBuilder::default()
    }

    pub(crate) fn new(mut builder: ServerBuilder) -> Self {
        let sockets = mem::take(&mut builder.sockets)
            .into_iter()
            .map(|t| (t.0, t.2))
            .collect();

        // Give log information on what runtime will be used.
        let is_tokio = tokio::runtime::Handle::try_current().is_ok();
        let is_actix = actix_rt::System::try_current().is_some();

        match (is_tokio, is_actix) {
            (true, false) => info!("Tokio runtime found. Starting in existing Tokio runtime"),
            (_, true) => info!("Actix runtime found. Starting in Actix runtime"),
            (_, _) => info!(
                "Actix/Tokio runtime not found. Starting in newt Tokio current-thread runtime"
            ),
        }

        for (_, name, lst) in &builder.sockets {
            info!(
                r#"Starting service: "{}", workers: {}, listening on: {}"#,
                name,
                builder.threads,
                lst.local_addr()
            );
        }

        match Accept::start(sockets, &builder) {
            Ok((waker_queue, worker_handles)) => {
                // construct OS signals listener future
                let signals = (builder.listen_os_signals).then(Signals::new);

                Self::Server(ServerInner {
                    cmd_tx: builder.cmd_tx.clone(),
                    cmd_rx: builder.cmd_rx,
                    signals,
                    waker_queue,
                    worker_handles,
                    worker_config: builder.worker_config,
                    services: builder.factories,
                    exit: builder.exit,
                    stop_task: None,
                })
            }

            Err(err) => Self::Error(Some(err)),
        }
    }

    /// Get a handle for ServerFuture that can be used to change state of actix server.
    ///
    /// See [ServerHandle](ServerHandle) for usage.
    pub fn handle(&self) -> ServerHandle {
        match self {
            Server::Server(inner) => ServerHandle::new(inner.cmd_tx.clone()),
            Server::Error(err) => {
                // TODO: i don't think this is the best way to handle server startup fail
                panic!(
                    "server handle can not be obtained because server failed to start up: {}",
                    err.as_ref().unwrap()
                );
            }
        }
    }
}

impl Future for Server {
    type Output = io::Result<()>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.as_mut().get_mut() {
            Server::Error(err) => Poll::Ready(Err(err
                .take()
                .expect("Server future cannot be polled after error"))),

            Server::Server(inner) => {
                // poll Signals
                if let Some(ref mut signals) = inner.signals {
                    if let Poll::Ready(signal) = Pin::new(signals).poll(cx) {
                        inner.stop_task = inner.handle_signal(signal);
                        // drop signals listener
                        inner.signals = None;
                    }
                }

                // handle stop tasks and eager drain command channel
                loop {
                    if let Some(ref mut fut) = inner.stop_task {
                        // only resolve stop task and exit
                        return fut.as_mut().poll(cx).map(|_| Ok(()));
                    }

                    match Pin::new(&mut inner.cmd_rx).poll_recv(cx) {
                        Poll::Ready(Some(cmd)) => {
                            // if stop task is required, set it and loop
                            inner.stop_task = inner.handle_cmd(cmd);
                        }
                        _ => return Poll::Pending,
                    }
                }
            }
        }
    }
}

/// Server handle.
#[derive(Debug, Clone)]
pub struct ServerHandle {
    tx_cmd: UnboundedSender<ServerCommand>,
}

impl ServerHandle {
    pub(crate) fn new(tx_cmd: UnboundedSender<ServerCommand>) -> Self {
        ServerHandle { tx_cmd }
    }

    pub(crate) fn worker_faulted(&self, idx: usize) {
        let _ = self.tx_cmd.send(ServerCommand::WorkerFaulted(idx));
    }

    /// Pause accepting incoming connections.
    ///
    /// May drop socket pending connection. All open connections remain active.
    pub fn pause(&self) -> impl Future<Output = ()> {
        let (tx, rx) = oneshot::channel();
        let _ = self.tx_cmd.send(ServerCommand::Pause(tx));
        async {
            let _ = rx.await;
        }
    }

    /// Resume accepting incoming connections.
    pub fn resume(&self) -> impl Future<Output = ()> {
        let (tx, rx) = oneshot::channel();
        let _ = self.tx_cmd.send(ServerCommand::Resume(tx));
        async {
            let _ = rx.await;
        }
    }

    /// Stop incoming connection processing, stop all workers and exit.
    pub fn stop(&self, graceful: bool) -> impl Future<Output = ()> {
        let (tx, rx) = oneshot::channel();
        let _ = self.tx_cmd.send(ServerCommand::Stop {
            graceful,
            completion: Some(tx),
        });
        async {
            let _ = rx.await;
        }
    }
}

pub struct ServerInner {
    worker_handles: Vec<WorkerHandleServer>,
    worker_config: ServerWorkerConfig,
    services: Vec<Box<dyn InternalServiceFactory>>,
    exit: bool,
    cmd_tx: UnboundedSender<ServerCommand>,
    cmd_rx: UnboundedReceiver<ServerCommand>,
    signals: Option<Signals>,
    waker_queue: WakerQueue,
    stop_task: Option<BoxFuture<'static, ()>>,
}

impl ServerInner {
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

            ServerCommand::Stop {
                graceful,
                completion,
            } => {
                let exit = self.exit;

                // stop accept thread
                self.waker_queue.wake(WakerInterest::Stop);

                // stop workers
                let workers_stop = self
                    .worker_handles
                    .iter()
                    .map(|worker| worker.stop(graceful))
                    .collect::<Vec<_>>();

                Some(Box::pin(async move {
                    if graceful {
                        // wait for all workers to shut down
                        let _ = join_all(workers_stop).await;
                    }

                    if let Some(tx) = completion {
                        let _ = tx.send(());
                    }

                    if exit {
                        sleep(Duration::from_millis(300)).await;
                        System::try_current().as_ref().map(System::stop);
                    }
                }))
            }

            ServerCommand::WorkerFaulted(idx) => {
                // TODO: maybe just return with warning log if not found ?
                assert!(self.worker_handles.iter().any(|wrk| wrk.idx == idx));

                error!("Worker {} has died; restarting", idx);

                let factories = self
                    .services
                    .iter()
                    .map(|service| service.clone_factory())
                    .collect();

                match ServerWorker::start(
                    idx,
                    factories,
                    self.waker_queue.clone(),
                    self.worker_config,
                ) {
                    Ok((handle_accept, handle_server)) => {
                        *self
                            .worker_handles
                            .iter_mut()
                            .find(|wrk| wrk.idx == idx)
                            .unwrap() = handle_server;

                        self.waker_queue.wake(WakerInterest::Worker(handle_accept));
                    }

                    Err(err) => error!("can not restart worker {}: {}", idx, err),
                };

                None
            }
        }
    }

    fn handle_signal(&mut self, signal: Signal) -> Option<BoxFuture<'static, ()>> {
        match signal {
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
}
