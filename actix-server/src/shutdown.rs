use std::future::pending;

use tokio::sync::watch;

/// Notification that the server has started a graceful shutdown.
///
/// The signal remains notified after graceful shutdown starts. A listener that calls
/// [`notified`](Self::notified) or clones this signal after that point is notified immediately.
/// A forced shutdown does not notify this signal.
///
/// # Server State
///
/// When this signal is notified, the [`Server`](crate::Server) has accepted a graceful shutdown
/// command, but shutdown is not complete. The server sends this notification before it tells the
/// accept loop and workers to stop. Therefore, a listener can briefly overlap with connection
/// acceptance and normal worker operation.
///
/// Immediately after notification, the server stops accepting connections and asks each worker to
/// stop its services. Active service futures can continue until they finish or the
/// [`ServerBuilder::shutdown_timeout`](crate::ServerBuilder::shutdown_timeout) expires.
///
/// # Use Cases
///
/// Connection-oriented services can listen to this signal to coordinate their own drain process.
/// For example, a protocol dispatcher can:
///
/// - close an idle persistent connection;
/// - stop accepting new logical requests on an active connection;
/// - let the current request or protocol operation finish; and
/// - start a protocol-specific close handshake or flush buffered data.
///
/// This signal does not replace the worker shutdown timeout. A listener must still finish its
/// service future for the worker to complete a graceful shutdown.
#[derive(Clone, Debug)]
pub struct GracefulShutdownSignal {
    rx: watch::Receiver<bool>,
}

impl GracefulShutdownSignal {
    pub(crate) fn new(rx: watch::Receiver<bool>) -> Self {
        Self { rx }
    }

    /// Resolves when the server starts a graceful shutdown.
    pub async fn notified(&self) {
        let mut rx = self.rx.clone();

        loop {
            if *rx.borrow_and_update() {
                return;
            }

            if rx.changed().await.is_err() {
                pending::<()>().await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use actix_rt::time::timeout;

    use super::GracefulShutdownSignal;

    #[actix_rt::test]
    async fn set_signal_notifies_listener() {
        let (tx, rx) = tokio::sync::watch::channel(false);
        let signal = GracefulShutdownSignal::new(rx);

        tx.send_replace(true);

        timeout(Duration::from_millis(100), signal.notified())
            .await
            .expect("set signal did not notify listener");
    }

    #[actix_rt::test]
    async fn closed_unset_signal_does_not_notify_listener() {
        let (tx, rx) = tokio::sync::watch::channel(false);
        let signal = GracefulShutdownSignal::new(rx);

        drop(tx);

        assert!(timeout(Duration::from_millis(10), signal.notified())
            .await
            .is_err());
    }
}
