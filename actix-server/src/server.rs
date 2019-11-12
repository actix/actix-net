use futures::channel::mpsc::UnboundedSender;
use futures::channel::oneshot;
use futures::{Future, TryFutureExt};

use crate::builder::ServerBuilder;
// use crate::signals::Signal;

#[derive(Debug)]
pub(crate) enum ServerCommand {
    WorkerFaulted(usize),
    Pause(oneshot::Sender<()>),
    Resume(oneshot::Sender<()>),
    // Signal(Signal),
    /// Whether to try and shut down gracefully
    Stop {
        graceful: bool,
        completion: Option<oneshot::Sender<()>>,
    },
}

#[derive(Debug, Clone)]
pub struct Server(UnboundedSender<ServerCommand>);

impl Server {
    pub(crate) fn new(tx: UnboundedSender<ServerCommand>) -> Self {
        Server(tx)
    }

    /// Start server building process
    pub fn build() -> ServerBuilder {
        ServerBuilder::default()
    }

    // pub(crate) fn signal(&self, sig: Signal) {
    //     let _ = self.0.unbounded_send(ServerCommand::Signal(sig));
    // }

    pub(crate) fn worker_faulted(&self, idx: usize) {
        let _ = self.0.unbounded_send(ServerCommand::WorkerFaulted(idx));
    }

    /// Pause accepting incoming connections
    ///
    /// If socket contains some pending connection, they might be dropped.
    /// All opened connection remains active.
    pub fn pause(&self) -> impl Future<Output = Result<(), ()>> {
        let (tx, rx) = oneshot::channel();
        let _ = self.0.unbounded_send(ServerCommand::Pause(tx));
        rx.map_err(|_| ())
    }

    /// Resume accepting incoming connections
    pub fn resume(&self) -> impl Future<Output = Result<(), ()>> {
        let (tx, rx) = oneshot::channel();
        let _ = self.0.unbounded_send(ServerCommand::Resume(tx));
        rx.map_err(|_| ())
    }

    /// Stop incoming connection processing, stop all workers and exit.
    ///
    /// If server starts with `spawn()` method, then spawned thread get terminated.
    pub fn stop(&self, graceful: bool) -> impl Future<Output = Result<(), ()>> {
        let (tx, rx) = oneshot::channel();
        let _ = self.0.unbounded_send(ServerCommand::Stop {
            graceful,
            completion: Some(tx),
        });
        rx.map_err(|_| ())
    }
}
