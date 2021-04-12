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
pub struct Server {
    cmd_tx: UnboundedSender<ServerCommand>,
    cmd_rx: UnboundedReceiver<ServerCommand>,
    handles: Vec<(usize, WorkerHandleServer)>,
    services: Vec<Box<dyn InternalServiceFactory>>,
    notify: Vec<oneshot::Sender<()>>,
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

        // start accept thread. return waker_queue and worker handles.
        let (waker_queue, handles) = Accept::start(sockets, &builder)
            // TODO: include error to Server type and poll return it in Future.
            .unwrap_or_else(|e| panic!("Can not start Accept: {}", e));

        // construct signals future.
        let signals = if !builder.no_signals {
            // Check tokio runtime.
            tokio::runtime::Handle::try_current()
                .map(|_|())
                .expect("there is no reactor running. Please enable ServerBuilder::disable_signals when start server in non tokio 1.x runtime.");
            Some(Signals::new())
        } else {
            None
        };

        Self {
            cmd_tx: builder.cmd_tx,
            cmd_rx: builder.cmd_rx,
            handles,
            services: builder.services,
            notify: Vec::new(),
            exit: builder.exit,
            worker_config: builder.worker_config,
            signals,
            on_stop_task: None,
            waker_queue,
        }
    }

    /// Obtain a Handle for ServerFuture that can be used to change state of actix server.
    ///
    /// See [ServerHandle](ServerHandle) for usage.
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
                            sleep(Duration::from_millis(300)).await;
                            System::try_current().as_ref().map(System::stop);
                        }
                    }))
                } else {
                    // we need to stop system if server was spawned
                    // TODO: this async block can return io::Error.
                    Some(Box::pin(async move {
                        if exit {
                            sleep(Duration::from_millis(300)).await;
                            System::try_current().as_ref().map(System::stop);
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
                    let res = ServerWorker::start(
                        new_idx,
                        factories,
                        availability,
                        self.worker_config,
                    );

                    match res {
                        Ok((handle_accept, handle_server)) => {
                            self.handles.push((new_idx, handle_server));
                            self.waker_queue.wake(WakerInterest::Worker(handle_accept));
                        }
                        Err(e) => error!("Can not start worker: {:?}", e),
                    }
                }

                None
            }
        }
    }
}

impl Future for Server {
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

#[derive(Debug)]
pub(crate) enum ServerCommand {
    WorkerFaulted(usize),
    Pause(oneshot::Sender<()>),
    Resume(oneshot::Sender<()>),
    Signal(Signal),
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
