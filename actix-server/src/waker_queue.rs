use std::{
    collections::VecDeque,
    ops::Deref,
    sync::{Arc, Mutex, MutexGuard},
};

use mio::{Registry, Token as MioToken, Waker};

use crate::worker::WorkerHandleAccept;

/// Waker token for `mio::Poll` instance.
pub(crate) const WAKER_TOKEN: MioToken = MioToken(usize::MAX);

/// `mio::Waker` with a queue for waking up the `Accept`'s `Poll` and contains the `WakerInterest`
/// the `Poll` would want to look into.
pub(crate) struct WakerQueue(Arc<(Waker, Mutex<VecDeque<WakerInterest>>)>);

impl Clone for WakerQueue {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl Deref for WakerQueue {
    type Target = (Waker, Mutex<VecDeque<WakerInterest>>);

    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

impl WakerQueue {
    /// Construct a waker queue with given `Poll`'s `Registry` and capacity.
    ///
    /// A fixed `WAKER_TOKEN` is used to identify the wake interest and the `Poll` needs to match
    /// event's token for it to properly handle `WakerInterest`.
    pub(crate) fn new(registry: &Registry) -> std::io::Result<Self> {
        let waker = Waker::new(registry, WAKER_TOKEN)?;
        let queue = Mutex::new(VecDeque::with_capacity(16));

        Ok(Self(Arc::new((waker, queue))))
    }

    /// Push a new interest to the queue and wake up the accept poll afterwards.
    pub(crate) fn wake(&self, interest: WakerInterest) {
        let (waker, queue) = self.deref();

        queue
            .lock()
            .expect("Failed to lock WakerQueue")
            .push_back(interest);

        waker
            .wake()
            .unwrap_or_else(|e| panic!("can not wake up Accept Poll: {}", e));
    }

    /// Get a MutexGuard of the waker queue.
    pub(crate) fn guard(&self) -> MutexGuard<'_, VecDeque<WakerInterest>> {
        self.deref().1.lock().expect("Failed to lock WakerQueue")
    }

    /// Reset the waker queue so it does not grow infinitely.
    pub(crate) fn reset(queue: &mut VecDeque<WakerInterest>) {
        std::mem::swap(&mut VecDeque::<WakerInterest>::with_capacity(16), queue);
    }
}

/// Types of interests we would look into when `Accept`'s `Poll` is waked up by waker.
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
    /// `Worker` is an interest that is triggered after a worker faults. This is determined by
    /// trying to send work to it. `Accept` would be waked up and add the new `WorkerHandleAccept`.
    Worker(WorkerHandleAccept),
}
