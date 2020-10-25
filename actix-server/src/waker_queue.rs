use std::ops::Deref;
use std::sync::Arc;

use concurrent_queue::{ConcurrentQueue, PopError};
use mio::{Registry, Token as MioToken, Waker};

use crate::worker::WorkerHandle;

/// waker token for `mio::Poll` instance
pub(crate) const WAKER_TOKEN: MioToken = MioToken(usize::MAX);

/// `mio::Waker` with a queue for waking up the `Accept`'s `Poll` and contains the `WakerInterest`
/// the `Poll` would want to look into.
pub(crate) struct WakerQueue(Arc<(Waker, ConcurrentQueue<WakerInterest>)>);

impl Clone for WakerQueue {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl Deref for WakerQueue {
    type Target = (Waker, ConcurrentQueue<WakerInterest>);

    fn deref(&self) -> &Self::Target {
        self.0.deref()
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
        let (waker, queue) = self.deref();

        // ToDo: should we handle error here?
        queue
            .push(interest)
            .unwrap_or_else(|e| panic!("WakerQueue overflow: {}", e));

        waker
            .wake()
            .unwrap_or_else(|e| panic!("can not wake up Accept Poll: {}", e));
    }

    /// pop an `WakerInterest` from the back of the queue.
    pub(crate) fn pop(&self) -> Result<WakerInterest, WakerQueueError> {
        self.deref().1.pop()
    }
}

/// types of interests we would look into when `Accept`'s `Poll` is waked up by waker.
///
/// *. These interests should not be confused with `mio::Interest` and mostly not I/O related
pub(crate) enum WakerInterest {
    /// `WorkerAvailable` is an interest from `Worker` notifying `Accept` there is a worker
    /// available and can accept new tasks.
    WorkerAvailable,
    /// `Pause`, `Resume`, `Stop` Interest are from `ServerBuilder` future. It listens to
    /// `ServerCommand` and notify `Accept` to do exactly these tasks.
    Pause,
    Resume,
    Stop,
    /// `Timer` is an interest sent as a delayed future. When an error happens on accepting
    /// connection `Accept` would deregister socket listener temporary and wake up the poll and
    /// register them again after the delayed future resolve.
    Timer,
    /// `WorkerNew` is an interest happen after a worker runs into faulted state(This is determined
    /// by if work can be sent to it successfully).`Accept` would be waked up and add the new
    /// `WorkerHandle`.
    Worker(WorkerHandle),
}

pub(crate) type WakerQueueError = PopError;
