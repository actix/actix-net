use std::future::Future;

use tokio::sync::{mpsc::UnboundedSender, oneshot};

use crate::server::ServerCommand;

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
