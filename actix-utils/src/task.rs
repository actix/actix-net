use core::cell::Cell;
use core::fmt;
use core::marker::PhantomData;
use core::task::Waker;

/// A synchronization primitive for task wakeup.
///
/// Sometimes the task interested in a given event will change over time.
/// An `LocalWaker` can coordinate concurrent notifications with the consumer
/// potentially "updating" the underlying task to wake up. This is useful in
/// scenarios where a computation completes in another task and wants to
/// notify the consumer, but the consumer is in the process of being migrated to
/// a new logical task.
///
/// Consumers should call `register` before checking the result of a computation
/// and producers should call `wake` after producing the computation (this
/// differs from the usual `thread::park` pattern). It is also permitted for
/// `wake` to be called **before** `register`. This results in a no-op.
///
/// A single `AtomicWaker` may be reused for any number of calls to `register` or
/// `wake`.
#[derive(Default)]
pub struct LocalWaker {
    pub(crate) waker: Cell<Option<Waker>>,
    // mark LocalWaker as a !Send type.
    _t: PhantomData<*const ()>,
}

impl LocalWaker {
    /// Create an `LocalWaker`.
    pub fn new() -> Self {
        LocalWaker {
            waker: Cell::new(None),
            _t: PhantomData,
        }
    }

    /// Registers the waker to be notified on calls to `wake`.
    ///
    /// Returns `true` if waker was registered before.
    #[inline]
    pub fn register(&self, waker: &Waker) -> bool {
        let last_waker = self.waker.replace(Some(waker.clone()));
        last_waker.is_some()
    }

    /// Calls `wake` on the last `Waker` passed to `register`.
    ///
    /// If `register` has not been called yet, then this does nothing.
    #[inline]
    pub fn wake(&self) {
        if let Some(waker) = self.take() {
            waker.wake();
        }
    }

    /// Returns the last `Waker` passed to `register`, so that the user can wake it.
    ///
    /// If a waker has not been registered, this returns `None`.
    #[inline]
    pub fn take(&self) -> Option<Waker> {
        self.waker.take()
    }
}

impl fmt::Debug for LocalWaker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "LocalWaker")
    }
}
