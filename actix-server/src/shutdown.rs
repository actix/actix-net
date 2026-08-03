use std::future::pending;

use tokio::sync::watch;

/// Notification that the server has started a graceful shutdown.
///
/// This signal is sticky. A listener created after shutdown starts is notified immediately.
#[derive(Clone, Debug)]
pub struct GracefulShutdownSignal {
    receiver: watch::Receiver<bool>,
}

impl GracefulShutdownSignal {
    pub(crate) fn new(receiver: watch::Receiver<bool>) -> Self {
        Self { receiver }
    }

    /// Resolves when the server starts a graceful shutdown.
    pub async fn notified(&self) {
        let mut receiver = self.receiver.clone();

        loop {
            if *receiver.borrow_and_update() {
                return;
            }

            if receiver.changed().await.is_err() {
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
        let (sender, receiver) = tokio::sync::watch::channel(false);
        let signal = GracefulShutdownSignal::new(receiver);

        sender.send_replace(true);

        timeout(Duration::from_millis(100), signal.notified())
            .await
            .expect("set signal did not notify listener");
    }

    #[actix_rt::test]
    async fn closed_unset_signal_does_not_notify_listener() {
        let (sender, receiver) = tokio::sync::watch::channel(false);
        let signal = GracefulShutdownSignal::new(receiver);

        drop(sender);

        assert!(timeout(Duration::from_millis(10), signal.notified())
            .await
            .is_err());
    }
}
