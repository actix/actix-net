use std::rc::Rc;

#[cfg(_loom)]
use loom::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

#[cfg(not(_loom))]
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use crate::waker_queue::{WakerInterest, WakerQueue};

/// counter: Arc<AtomicUsize> field is owned by `Accept` thread and `ServerWorker` thread.
///
/// `Accept` would increment the counter and `ServerWorker` would decrement it.
///
/// # Atomic Ordering:
///
/// `Accept` always look into it's cached `Availability` field for `ServerWorker` state.
/// It lazily increment counter after successful dispatching new work to `ServerWorker`.
/// On reaching counter limit `Accept` update it's cached `Availability` and mark worker as
/// unable to accept any work.
///
/// `ServerWorker` always decrement the counter when every work received from `Accept` is done.
/// On reaching counter limit worker would use `mio::Waker` and `WakerQueue` to wake up `Accept`
/// and notify it to update cached `Availability` again to mark worker as able to accept work again.
///
/// Hense a wake up would only happen after `Accept` increment it to limit.
/// And a decrement to limit always wake up `Accept`.
#[derive(Clone)]
pub(crate) struct Counter {
    counter: Arc<AtomicUsize>,
    limit: usize,
}

impl Counter {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            counter: Arc::new(AtomicUsize::new(1)),
            limit,
        }
    }

    /// Increment counter by 1 and return true when hitting limit
    #[inline(always)]
    pub(crate) fn inc(&self) -> bool {
        self.counter.fetch_add(1, Ordering::Relaxed) != self.limit
    }

    /// Decrement counter by 1 and return true if crossing limit.
    #[inline(always)]
    pub(crate) fn dec(&self) -> bool {
        self.counter.fetch_sub(1, Ordering::Relaxed) == self.limit
    }

    pub(crate) fn total(&self) -> usize {
        self.counter.load(Ordering::SeqCst) - 1
    }
}

pub(super) struct WorkerCounter<C> {
    idx: usize,
    inner: Rc<(WakerQueue<C>, Counter)>,
}

impl<C> Clone for WorkerCounter<C> {
    fn clone(&self) -> Self {
        Self {
            idx: self.idx,
            inner: self.inner.clone(),
        }
    }
}

impl<C> WorkerCounter<C> {
    pub(super) fn new(idx: usize, waker_queue: WakerQueue<C>, counter: Counter) -> Self {
        Self {
            idx,
            inner: Rc::new((waker_queue, counter)),
        }
    }

    #[inline(always)]
    pub(super) fn guard(&self) -> WorkerCounterGuard<C> {
        WorkerCounterGuard(self.clone())
    }

    pub(super) fn total(&self) -> usize {
        self.inner.1.total()
    }
}

pub(crate) struct WorkerCounterGuard<C>(WorkerCounter<C>);

impl<C> Drop for WorkerCounterGuard<C> {
    fn drop(&mut self) {
        let (waker_queue, counter) = &*self.0.inner;
        if counter.dec() {
            waker_queue.wake(WakerInterest::WorkerAvailable(self.0.idx));
        }
    }
}
