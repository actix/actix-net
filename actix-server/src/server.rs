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
use tokio::sync::{mpsc::UnboundedReceiver, oneshot};

use crate::{
    accept::Accept,
    builder::ServerBuilder,
    join_all::join_all,
    service::InternalServiceFactory,
    signals::{SignalKind, Signals},
    waker_queue::{WakerInterest, WakerQueue},
    worker::{ServerWorker, ServerWorkerConfig, WorkerHandleServer},
    ServerHandle,
};

#[derive(Debug)]
pub(crate) enum ServerCommand {
    /// Worker failed to accept connection, indicating a probable panic.
    ///
    /// Contains index of faulted worker.
    WorkerFaulted(usize),

    /// Pause accepting connections.
    ///
    /// Contains return channel to notify caller of successful state change.
    Pause(oneshot::Sender<()>),

    /// Resume accepting connections.
    ///
    /// Contains return channel to notify caller of successful state change.
    Resume(oneshot::Sender<()>),

    /// Stop accepting connections and begin shutdown procedure.
    Stop {
        /// True if shut down should be graceful.
        graceful: bool,

        /// Return channel to notify caller that shutdown is complete.
        completion: Option<oneshot::Sender<()>>,
    },
}

/// General purpose TCP server that runs services receiving Tokio `TcpStream`s.
///
/// Handles creating worker threads, restarting faulted workers, connection accepting, and
/// back-pressure logic.
///
/// Creates a worker per CPU core (or the number specified in [`ServerBuilder::workers`]) and
/// distributes connections with a round-robin strategy.
///
/// The [Server] must be awaited to process stop commands and listen for OS signals. It will resolve
/// when the server has fully shut down.
///
/// # Shutdown Signals
/// On UNIX systems, `SIGQUIT` will start a graceful shutdown and `SIGTERM` or `SIGINT` will start a
/// forced shutdown. On Windows, a Ctrl-C signal will start a forced shutdown.
///
/// A graceful shutdown will wait for all workers to stop first.
///
/// # Examples
/// The following is a TCP echo server. Test using `telnet 127.0.0.1 8080`.
///
/// ```no_run
/// use std::io;
///
/// use actix_rt::net::TcpStream;
/// use actix_server::Server;
/// use actix_service::{fn_service, ServiceFactoryExt as _};
/// use bytes::BytesMut;
/// use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
///
/// #[actix_rt::main]
/// async fn main() -> io::Result<()> {
///     let bind_addr = ("127.0.0.1", 8080);
///
///     Server::build()
///         .bind("echo", bind_addr, move || {
///             fn_service(move |mut stream: TcpStream| {
///                 async move {
///                     let mut size = 0;
///                     let mut buf = BytesMut::new();
///
///                     loop {
///                         match stream.read_buf(&mut buf).await {
///                             // end of stream; bail from loop
///                             Ok(0) => break,
///
///                             // write bytes back to stream
///                             Ok(bytes_read) => {
///                                 stream.write_all(&buf[size..]).await.unwrap();
///                                 size += bytes_read;
///                             }
///
///                             Err(err) => {
///                                 eprintln!("Stream Error: {:?}", err);
///                                 return Err(());
///                             }
///                         }
///                     }
///
///                     Ok(())
///                 }
///             })
///             .map_err(|err| eprintln!("Service Error: {:?}", err))
///         })?
///         .run()
///         .await
/// }
/// ```
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct Server {
    handle: ServerHandle,
    fut: BoxFuture<'static, io::Result<()>>,
}

impl Server {
    /// Create server build.
    pub fn build() -> ServerBuilder {
        ServerBuilder::default()
    }

    pub(crate) fn new(builder: ServerBuilder) -> Self {
        Server {
            handle: ServerHandle::new(builder.cmd_tx.clone()),
            fut: Box::pin(ServerInner::run(builder)),
        }
    }

    /// Get a handle for ServerFuture that can be used to change state of actix server.
    ///
    /// See [ServerHandle](ServerHandle) for usage.
    pub fn handle(&self) -> ServerHandle {
        self.handle.clone()
    }
}

impl Future for Server {
    type Output = io::Result<()>;

    #[inline]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut Pin::into_inner(self).fut).poll(cx)
    }
}

pub struct ServerInner {
    worker_handles: Vec<WorkerHandleServer>,
    worker_config: ServerWorkerConfig,
    services: Vec<Box<dyn InternalServiceFactory>>,
    exit: bool,
    waker_queue: WakerQueue,
    stopped: bool,
}

impl ServerInner {
    async fn run(builder: ServerBuilder) -> io::Result<()> {
        let (mut this, mut cmd_rx, mut signal_fut) = Self::run_sync(builder)?;
        let listen_to_signals = signal_fut.is_some();

        while !this.stopped {
            tokio::select! {
                signal = async {
                    signal_fut.as_mut().unwrap().await
                }, if listen_to_signals => {
                    this.handle_signal(signal).await;
                },
                Some(cmd) = cmd_rx.recv() => {
                    this.handle_cmd(cmd).await;
                },
                else => break,
            };
        }

        Ok(())
    }

    fn run_sync(
        mut builder: ServerBuilder,
    ) -> io::Result<(Self, UnboundedReceiver<ServerCommand>, Option<Signals>)> {
        let sockets = mem::take(&mut builder.sockets)
            .into_iter()
            .map(|t| (t.0, t.2))
            .collect();

        // Give log information on what runtime will be used.
        let is_actix = actix_rt::System::try_current().is_some();
        let is_tokio = tokio::runtime::Handle::try_current().is_ok();

        match (is_actix, is_tokio) {
            (true, _) => info!("Actix runtime found; starting in Actix runtime"),
            (_, true) => info!("Tokio runtime found; starting in existing Tokio runtime"),
            (_, false) => panic!("Actix or Tokio runtime not found; halting"),
        }

        for (_, name, lst) in &builder.sockets {
            info!(
                r#"Starting service: "{}", workers: {}, listening on: {}"#,
                name,
                builder.threads,
                lst.local_addr()
            );
        }

        let (waker_queue, worker_handles) = Accept::start(sockets, &builder)?;

        // construct OS signals listener future
        let signal_fut = (builder.listen_os_signals).then(Signals::new);

        let server = ServerInner {
            waker_queue,
            worker_handles,
            worker_config: builder.worker_config,
            services: builder.factories,
            exit: builder.exit,
            stopped: false,
        };

        Ok((server, builder.cmd_rx, signal_fut))
    }

    async fn handle_cmd(&mut self, item: ServerCommand) {
        match item {
            ServerCommand::Pause(tx) => {
                self.waker_queue.wake(WakerInterest::Pause);
                let _ = tx.send(());
            }

            ServerCommand::Resume(tx) => {
                self.waker_queue.wake(WakerInterest::Resume);
                let _ = tx.send(());
            }

            ServerCommand::Stop {
                graceful,
                completion,
            } => {
                self.stopped = true;

                // stop accept thread
                self.waker_queue.wake(WakerInterest::Stop);

                if graceful {
                    // wait for all workers to shut down
                    let workers_stop = self
                        .worker_handles
                        .iter()
                        .map(|worker| worker.stop(graceful))
                        .collect::<Vec<_>>();
                    let _ = join_all(workers_stop).await;
                }

                if let Some(tx) = completion {
                    let _ = tx.send(());
                }

                if self.exit {
                    sleep(Duration::from_millis(300)).await;
                    System::try_current().as_ref().map(System::stop);
                }
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
            }
        }
    }

    async fn handle_signal(&mut self, signal: SignalKind) {
        match signal {
            SignalKind::Int => {
                info!("SIGINT received; starting forced shutdown");
                self.exit = true;
                self.handle_cmd(ServerCommand::Stop {
                    graceful: false,
                    completion: None,
                })
                .await
            }

            SignalKind::Term => {
                info!("SIGTERM received; starting graceful shutdown");
                self.exit = true;
                self.handle_cmd(ServerCommand::Stop {
                    graceful: true,
                    completion: None,
                })
                .await
            }

            SignalKind::Quit => {
                info!("SIGQUIT received; starting forced shutdown");
                self.exit = true;
                self.handle_cmd(ServerCommand::Stop {
                    graceful: false,
                    completion: None,
                })
                .await
            }
        }
    }
}
