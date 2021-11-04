use std::future::Future;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::oneshot;

use crate::builder::ServerBuilder;
use crate::signals::Signal;

#[derive(Debug)]
pub(crate) enum ServerCommand {
    WorkerFaulted(usize),
    Pause(oneshot::Sender<()>),
    Resume(oneshot::Sender<()>),
    Signal(Signal),
    Stop {
        /// True if shut down should be graceful.
        graceful: bool,
        completion: Option<oneshot::Sender<()>>,
    },
    /// Notify of server stop
    Notify(oneshot::Sender<()>),
}

#[derive(Debug)]
#[non_exhaustive]
pub struct Server;

impl Server {
    /// Start server building process.
    pub fn build() -> ServerBuilder {
        ServerBuilder::default()
    }
}

/// Server handle.
///
/// # Shutdown Signals
/// On UNIX systems, `SIGQUIT` will start a graceful shutdown and `SIGTERM` or `SIGINT` will start a
/// forced shutdown. On Windows, a CTRL-C signal will start a forced shutdown.
///
/// A graceful shutdown will wait for all workers to stop first.
#[derive(Debug)]
pub struct ServerHandle(
    UnboundedSender<ServerCommand>,
    Option<oneshot::Receiver<()>>,
);

impl ServerHandle {
    pub(crate) fn new(tx: UnboundedSender<ServerCommand>) -> Self {
        ServerHandle(tx, None)
    }

    pub(crate) fn signal(&self, sig: Signal) {
        let _ = self.0.send(ServerCommand::Signal(sig));
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

impl Clone for ServerHandle {
    fn clone(&self) -> Self {
        Self(self.0.clone(), None)
    }
}

impl Future for ServerHandle {
    type Output = io::Result<()>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        if this.1.is_none() {
            let (tx, rx) = oneshot::channel();
            if this.0.send(ServerCommand::Notify(tx)).is_err() {
                return Poll::Ready(Ok(()));
            }
            this.1 = Some(rx);
        }

        match Pin::new(this.1.as_mut().unwrap()).poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(_) => Poll::Ready(Ok(())),
        }
    }
}
