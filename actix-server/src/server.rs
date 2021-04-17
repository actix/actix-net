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

use crate::accept::Accept;
use crate::builder::ServerBuilder;
use crate::service::InternalServiceFactory;
use crate::signals::{Signal, Signals};
use crate::waker_queue::{WakerInterest, WakerQueue};
use crate::worker::{ServerWorker, ServerWorkerConfig, WorkerAvailability, WorkerHandleServer};

/// When awaited or spawned would listen to signal and message from [ServerHandle](ServerHandle).
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub enum Server {
    Server(ServerInner),
    Error(Option<io::Error>),
}

pub struct ServerInner {
    cmd_tx: UnboundedSender<ServerCommand>,
    cmd_rx: UnboundedReceiver<ServerCommand>,
    handles: Vec<WorkerHandleServer>,
    services: Vec<Box<dyn InternalServiceFactory>>,
    exit: bool,
    worker_config: ServerWorkerConfig,
    signals: Option<Signals>,
    on_stop_task: Option<BoxFuture<'static, ()>>,
    waker_queue: WakerQueue,
}

impl Server {
    /// Start server building process
    pub fn build() -> ServerBuilder {
        ServerBuilder::default()
    }

    pub(crate) fn new(mut builder: ServerBuilder) -> Self {
        let sockets = mem::take(&mut builder.sockets)
            .into_iter()
            .map(|(token, name, lst)| {
                info!("Starting \"{}\" service on {}", name, lst);
                (token, lst)
            })
            .collect();

        // Give log information on what runtime will be used.
        let is_tokio = tokio::runtime::Handle::try_current().is_ok();
        let is_actix = actix_rt::System::try_current().is_some();

        if is_tokio && !is_actix {
            info!("Tokio runtime found. Starting in existing tokio runtime");
        } else if is_actix {
            info!("Actix runtime found. Starting in actix runtime");
        } else {
            info!(
                "Actix/Tokio runtime not found. Starting in new tokio current-thread runtime"
            );
        }

        // start accept thread. return waker_queue and worker handles.
        match Accept::start(sockets, &builder) {
            Ok((waker_queue, handles)) => {
                // construct signals future.
                let signals = if !builder.no_signals {
                    // Check tokio runtime.
                    if !is_tokio {
                        let err = io::Error::new(io::ErrorKind::Other, "there is no reactor running. Please enable ServerBuilder::disable_signals when start server in non tokio 1.x runtime.");
                        return Self::Error(Some(err));
                    }
                    Some(Signals::new())
                } else {
                    None
                };

                Self::Server(ServerInner {
                    cmd_tx: builder.cmd_tx,
                    cmd_rx: builder.cmd_rx,
                    handles,
                    services: builder.services,
                    exit: builder.exit,
                    worker_config: builder.worker_config,
                    signals,
                    on_stop_task: None,
                    waker_queue,
                })
            }
            Err(e) => Self::Error(Some(e)),
        }
    }

    /// Obtain a Handle for ServerFuture that can be used to change state of actix server.
    ///
    /// See [ServerHandle](ServerHandle) for usage.
    pub fn handle(&self) -> ServerHandle {
        match self {
            Self::Server(ref inner) => ServerHandle::new(inner.cmd_tx.clone()),
            Self::Error(err) => panic!(
                "ServerHandle can not be obtained. Server failed to start due to error: {:?}",
                err.as_ref().unwrap()
            ),
        }
    }
}

impl ServerInner {
    fn handle_signal(&mut self, signal: Signal) -> Option<BoxFuture<'static, ()>> {
        // Signals support
        // Handle `SIGINT`, `SIGTERM`, `SIGQUIT` signals and stop actix system
        match signal {
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
                let stop = self
                    .handles
                    .iter()
                    .map(move |worker| worker.stop(graceful))
                    .collect::<Vec<_>>();

                // TODO: this async block can return io::Error.
                Some(Box::pin(async move {
                    if graceful {
                        for handle in stop {
                            let _ = handle.await;
                        }
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
                assert!(self.handles.iter().any(|handle| handle.idx == idx));

                error!("Worker {} has died, restarting", idx);

                let availability = WorkerAvailability::new(idx, self.waker_queue.clone());
                let factories = self
                    .services
                    .iter()
                    .map(|service| service.clone_factory())
                    .collect();

                match ServerWorker::start(idx, factories, availability, self.worker_config) {
                    Ok((handle_accept, handle_server)) => {
                        *self
                            .handles
                            .iter_mut()
                            .find(|handle| handle.idx == idx)
                            .unwrap() = handle_server;
                        self.waker_queue.wake(WakerInterest::Worker(handle_accept));
                    }
                    Err(e) => error!("Can not start worker: {:?}", e),
                }

                None
            }
        }
    }
}

impl Future for Server {
    type Output = io::Result<()>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.as_mut().get_mut() {
            Self::Error(e) => Poll::Ready(Err(e.take().unwrap())),
            Self::Server(inner) => {
                // poll signals first. remove it on resolve.
                if let Some(ref mut signals) = inner.signals {
                    if let Poll::Ready(signal) = Pin::new(signals).poll(cx) {
                        inner.on_stop_task = inner.handle_signal(signal);
                        inner.signals = None;
                    }
                }

                // actively poll command channel and handle command.
                loop {
                    // got on stop task. resolve it exclusively and exit.
                    if let Some(ref mut fut) = inner.on_stop_task {
                        return fut.as_mut().poll(cx).map(|_| Ok(()));
                    }

                    match Pin::new(&mut inner.cmd_rx).poll_recv(cx) {
                        Poll::Ready(Some(it)) => {
                            inner.on_stop_task = inner.handle_cmd(it);
                        }
                        _ => return Poll::Pending,
                    }
                }
            }
        }
    }
}

#[derive(Debug)]
pub(crate) enum ServerCommand {
    WorkerFaulted(usize),
    Pause(oneshot::Sender<()>),
    Resume(oneshot::Sender<()>),
    /// Whether to try and shut down gracefully
    Stop {
        graceful: bool,
        completion: Option<oneshot::Sender<()>>,
    },
}

#[derive(Clone, Debug)]
pub struct ServerHandle(UnboundedSender<ServerCommand>);

impl ServerHandle {
    pub(crate) fn new(tx: UnboundedSender<ServerCommand>) -> Self {
        ServerHandle(tx)
    }

    pub(crate) fn worker_faulted(&self, idx: usize) {
        let _ = self.0.send(ServerCommand::WorkerFaulted(idx));
    }

    /// Pause accepting incoming connections
    ///
    /// If socket contains some pending connection, they might be dropped.
    /// All opened connection remains active.
    pub fn pause(&self) -> impl Future<Output = ()> {
        let (tx, rx) = oneshot::channel();
        let _ = self.0.send(ServerCommand::Pause(tx));
        async {
            let _ = rx.await;
        }
    }

    /// Resume accepting incoming connections
    pub fn resume(&self) -> impl Future<Output = ()> {
        let (tx, rx) = oneshot::channel();
        let _ = self.0.send(ServerCommand::Resume(tx));
        async {
            let _ = rx.await;
        }
    }

    /// Stop incoming connection processing, stop all workers and exit.
    ///
    /// If server starts with `spawn()` method, then spawned thread get terminated.
    pub fn stop(&self, graceful: bool) -> impl Future<Output = ()> {
        let (tx, rx) = oneshot::channel();
        let _ = self.0.send(ServerCommand::Stop {
            graceful,
            completion: Some(tx),
        });
        async {
            let _ = rx.await;
        }
    }
}
