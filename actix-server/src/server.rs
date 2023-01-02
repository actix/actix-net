use std::{
    future::Future,
    io, mem,
    pin::Pin,
    task::{Context, Poll},
    thread,
    time::Duration,
};

use actix_rt::{time::sleep, System};
use futures_core::{future::BoxFuture, Stream};
use futures_util::stream::StreamExt as _;
use tokio::sync::{mpsc::UnboundedReceiver, oneshot};
use tracing::{error, info};

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

        /// Force System exit when true, overriding `ServerBuilder::system_exit()` if it is false.
        force_system_stop: bool,
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
/// The [Server] must be awaited or polled in order to start running. It will resolve when the
/// server has fully shut down.
///
/// # Shutdown Signals
/// On UNIX systems, `SIGTERM` will start a graceful shutdown and `SIGQUIT` or `SIGINT` will start a
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
#[must_use = "Server does nothing unless you `.await` or poll it"]
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

    /// Get a `Server` handle that can be used issue commands and change it's state.
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
    accept_handle: Option<thread::JoinHandle<()>>,
    worker_config: ServerWorkerConfig,
    services: Vec<Box<dyn InternalServiceFactory>>,
    waker_queue: WakerQueue,
    system_stop: bool,
    stopping: bool,
}

impl ServerInner {
    async fn run(builder: ServerBuilder) -> io::Result<()> {
        let (mut this, mut mux) = Self::run_sync(builder)?;

        while let Some(cmd) = mux.next().await {
            this.handle_cmd(cmd).await;

            if this.stopping {
                break;
            }
        }

        Ok(())
    }

    fn run_sync(mut builder: ServerBuilder) -> io::Result<(Self, ServerEventMultiplexer)> {
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
                r#"starting service: "{}", workers: {}, listening on: {}"#,
                name,
                builder.threads,
                lst.local_addr()
            );
        }

        let (waker_queue, worker_handles, accept_handle) = Accept::start(sockets, &builder)?;

        let mux = ServerEventMultiplexer {
            signal_fut: (builder.listen_os_signals).then(Signals::new),
            cmd_rx: builder.cmd_rx,
        };

        let server = ServerInner {
            waker_queue,
            accept_handle: Some(accept_handle),
            worker_handles,
            worker_config: builder.worker_config,
            services: builder.factories,
            system_stop: builder.exit,
            stopping: false,
        };

        Ok((server, mux))
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
                force_system_stop,
            } => {
                self.stopping = true;

                // Signal accept thread to stop.
                // Signal is non-blocking; we wait for thread to stop later.
                self.waker_queue.wake(WakerInterest::Stop);

                // send stop signal to workers
                let workers_stop = self
                    .worker_handles
                    .iter()
                    .map(|worker| worker.stop(graceful))
                    .collect::<Vec<_>>();

                if graceful {
                    // wait for all workers to shut down
                    let _ = join_all(workers_stop).await;
                }

                // wait for accept thread stop
                self.accept_handle
                    .take()
                    .unwrap()
                    .join()
                    .expect("Accept thread must not panic in any case");

                if let Some(tx) = completion {
                    let _ = tx.send(());
                }

                if self.system_stop || force_system_stop {
                    sleep(Duration::from_millis(300)).await;
                    System::try_current().as_ref().map(System::stop);
                }
            }

            ServerCommand::WorkerFaulted(idx) => {
                // TODO: maybe just return with warning log if not found ?
                assert!(self.worker_handles.iter().any(|wrk| wrk.idx == idx));

                error!("worker {} has died; restarting", idx);

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

    fn map_signal(signal: SignalKind) -> ServerCommand {
        match signal {
            SignalKind::Int => {
                info!("SIGINT received; starting forced shutdown");
                ServerCommand::Stop {
                    graceful: false,
                    completion: None,
                    force_system_stop: true,
                }
            }

            SignalKind::Term => {
                info!("SIGTERM received; starting graceful shutdown");
                ServerCommand::Stop {
                    graceful: true,
                    completion: None,
                    force_system_stop: true,
                }
            }

            SignalKind::Quit => {
                info!("SIGQUIT received; starting forced shutdown");
                ServerCommand::Stop {
                    graceful: false,
                    completion: None,
                    force_system_stop: true,
                }
            }
        }
    }
}

struct ServerEventMultiplexer {
    cmd_rx: UnboundedReceiver<ServerCommand>,
    signal_fut: Option<Signals>,
}

impl Stream for ServerEventMultiplexer {
    type Item = ServerCommand;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = Pin::into_inner(self);

        if let Some(signal_fut) = &mut this.signal_fut {
            if let Poll::Ready(signal) = Pin::new(signal_fut).poll(cx) {
                this.signal_fut = None;
                return Poll::Ready(Some(ServerInner::map_signal(signal)));
            }
        }

        this.cmd_rx.poll_recv(cx)
    }
}
