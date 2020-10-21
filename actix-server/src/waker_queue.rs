use std::sync::Arc;

use concurrent_queue::{ConcurrentQueue, PopError};
use mio::{Registry, Token as MioToken, Waker};

use crate::worker::WorkerClient;

/// waker token for `mio::Poll` instance
pub(crate) const WAKER_TOKEN: MioToken = MioToken(1);

/// `mio::Waker` with a queue for waking up the `Accept`'s `Poll` and contains the `WakerInterest`
/// we want `Poll` to look into.
pub(crate) struct WakerQueue(Arc<(Waker, ConcurrentQueue<WakerInterest>)>);

impl Clone for WakerQueue {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl WakerQueue {
    /// construct a waker queue with given `Poll`'s `Registry` and capacity.
    ///
    /// A fixed `WAKER_TOKEN` is used to identify the wake interest and the `Poll` needs to match
    /// event's token for it to properly handle `WakerInterest`.
    pub(crate) fn with_capacity(registry: &Registry, cap: usize) -> std::io::Result<Self> {
        let waker = Waker::new(registry, WAKER_TOKEN)?;
        let queue = ConcurrentQueue::bounded(cap);

        Ok(Self(Arc::new((waker, queue))))
    }

    /// push a new interest to the queue and wake up the accept poll afterwards.
    pub(crate) fn wake(&self, interest: WakerInterest) {
        // ToDo: should we handle error here?
        let r = (self.0).1.push(interest);
        assert!(r.is_ok());

        (self.0).0.wake().expect("can not wake up Accept Poll");
    }

    /// pop an `WakerInterest` from the back of the queue.
    pub(crate) fn pop(&self) -> Result<WakerInterest, WakerQueueError> {
        (self.0).1.pop()
    }
}

/// types of interests we would look into when `Accept`'s `Poll` is waked up by waker.
///
/// *. These interests should not be confused with `mio::Interest` and mostly not I/O related
pub(crate) enum WakerInterest {
    /// Interest from `Worker` notifying `Accept` to run `maybe_backpressure` method
    Notify,
    /// `Pause`, `Resume`, `Stop` Interest are from `ServerBuilder` future. It listens to
    /// `ServerCommand` and notify `Accept` to do exactly these tasks.
    Pause,
    Resume,
    Stop,
    /// `Timer` is an interest sent as a delayed future. When an error happens on accepting
    /// connection `Accept` would deregister sockets temporary and wake up the poll and register
    /// them again after the delayed future resolve.
    Timer,
    /// `Worker` ins an interest happen after a worker runs into faulted state(This is determined by
    /// if work can be sent to it successfully).`Accept` would be waked up and add the new
    /// `WorkerClient` to workers.
    Worker(WorkerClient),
}

pub(crate) type WakerQueueError = PopError;
