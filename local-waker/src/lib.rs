//! A synchronization primitive for thread-local task wakeup.
//!
//! See docs for [`LocalWaker`].

#![no_std]
#![deny(rust_2018_idioms, nonstandard_style)]
#![warn(future_incompatible, missing_docs)]

use core::{cell::UnsafeCell, fmt, marker::PhantomData, task::Waker};

/// A synchronization primitive for task wakeup.
///
/// Sometimes the task interested in a given event will change over time. A `LocalWaker` can
/// coordinate concurrent notifications with the consumer, potentially "updating" the underlying
/// task to wake up. This is useful in scenarios where a computation completes in another task and
/// wants to notify the consumer, but the consumer is in the process of being migrated to a new
/// logical task.
///
/// Consumers should call [`register`] before checking the result of a computation and producers
/// should call [`wake`] after producing the computation (this differs from the usual `thread::park`
/// pattern). It is also permitted for [`wake`] to be called _before_ [`register`]. This results in
/// a no-op.
///
/// A single `LocalWaker` may be reused for any number of calls to [`register`] or [`wake`].
///
/// [`register`]: LocalWaker::register
/// [`wake`]: LocalWaker::wake
#[derive(Default)]
pub struct LocalWaker {
    pub(crate) waker: UnsafeCell<Option<Waker>>,
    // mark LocalWaker as a !Send type.
    _phantom: PhantomData<*const ()>,
}

impl LocalWaker {
    /// Creates a new, empty `LocalWaker`.
    pub fn new() -> Self {
        LocalWaker::default()
    }

    /// Registers the waker to be notified on calls to `wake`.
    ///
    /// Returns `true` if waker was registered before.
    #[inline]
    pub fn register(&self, waker: &Waker) -> bool {
        let mut registered = false;
        if let Some(prev) = unsafe { &*self.waker.get() } {
            if waker.will_wake(prev) {
                return true;
            }
            registered = true;
        }
        unsafe { *self.waker.get() = Some(waker.clone()) }
        registered
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
        unsafe { (*self.waker.get()).take() }
    }
}

impl fmt::Debug for LocalWaker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "LocalWaker")
    }
}
