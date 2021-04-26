use std::{
    ops::{Deref, DerefMut},
    sync::Arc,
    task::{Wake, Waker},
};

use mio::{Registry, Token};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

use crate::worker::WorkerHandleAccept;

/// Waker token for `mio::Poll` instance.
pub(crate) const WAKER_TOKEN: Token = Token(usize::MAX);

/// Types of interests we would look into when `Accept`'s `Poll` is waked up by WakerTx.
///
/// These interests should not be confused with `mio::Interest` and mostly not I/O related
pub(crate) enum WakerInterest {
    /// `WorkerAvailable` is an interest from `Worker` notifying `Accept` there is a worker
    /// available and can accept new tasks.
    WorkerAvailable(usize),
    /// `Pause`, `Resume`, `Stop` Interest are from `ServerBuilder` future. It listens to
    /// `ServerCommand` and notify `Accept` to do exactly these tasks.
    Pause,
    Resume,
    Stop,
    /// `Worker` is an interest happen after a worker runs into faulted state(This is determined
    /// by if work can be sent to it successfully).`Accept` would be waked up and add the new
    /// `WorkerHandleAccept`.
    Worker(WorkerHandleAccept),
}

/// Wrapper type for mio::Waker in order to impl std::task::Wake trait.
struct _Waker(mio::Waker);

impl Wake for _Waker {
    fn wake(self: Arc<Self>) {
        Wake::wake_by_ref(&self)
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0
            .wake()
            .unwrap_or_else(|e| panic!("Can not wake up Accept Poll: {}", e));
    }
}

/// Wrapper type for tokio unbounded channel sender.
pub(crate) struct WakerTx(UnboundedSender<WakerInterest>);

impl Clone for WakerTx {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl WakerTx {
    pub(crate) fn wake(&self, interest: WakerInterest) {
        // ingore result. tokio UnboundedSender::send only fail when the
        // channel is closed.
        // In that case the Accept thread is gone and no further wake up
        // is needed/possible.
        let _ = self.0.send(interest);
    }
}

/// Wrapper type for tokio unbounded channel receiver.
pub(crate) struct WakerRx(UnboundedReceiver<WakerInterest>);

impl Deref for WakerRx {
    type Target = UnboundedReceiver<WakerInterest>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for WakerRx {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

pub(crate) fn from_registry(registry: &Registry) -> std::io::Result<Waker> {
    mio::Waker::new(registry, WAKER_TOKEN).map(|waker| Waker::from(Arc::new(_Waker(waker))))
}

pub(crate) fn waker_channel() -> (WakerTx, WakerRx) {
    let (tx, rx) = unbounded_channel();

    (WakerTx(tx), WakerRx(rx))
}

#[cfg(test)]
mod test {
    use std::task::{Context, Poll};

    use super::*;

    #[test]
    fn test_waker_channel() {
        let poll = mio::Poll::new().unwrap();

        let waker = from_registry(poll.registry()).unwrap();

        let cx = &mut Context::from_waker(&waker);

        let (tx, mut rx) = waker_channel();

        assert!(rx.poll_recv(cx).is_pending());

        tx.wake(super::WakerInterest::Stop);

        match rx.poll_recv(cx) {
            Poll::Ready(Some(WakerInterest::Stop)) => {}
            _ => panic!("Failed to wake up WakerRx"),
        }

        drop(tx);

        match rx.poll_recv(cx) {
            Poll::Ready(None) => {}
            _ => panic!("Failed to close waker channel"),
        }
    }
}
