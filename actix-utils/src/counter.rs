//! Task-notifying counter.

use core::{cell::Cell, fmt, task};
use std::rc::Rc;

use local_waker::LocalWaker;

/// Simple counter with ability to notify task on reaching specific number
///
/// Counter could be cloned, total n-count is shared across all clones.
#[derive(Debug, Clone)]
pub struct Counter(Rc<CounterInner>);

impl Counter {
    /// Create `Counter` instance with max value.
    pub fn new(capacity: usize) -> Self {
        Counter(Rc::new(CounterInner {
            capacity,
            count: Cell::new(0),
            task: LocalWaker::new(),
        }))
    }

    /// Create new counter guard, incrementing the counter.
    #[inline]
    pub fn get(&self) -> CounterGuard {
        CounterGuard::new(self.0.clone())
    }

    /// Returns true if counter is below capacity. Otherwise, register to wake task when it is.
    #[inline]
    pub fn available(&self, cx: &mut task::Context<'_>) -> bool {
        self.0.available(cx)
    }

    /// Get total number of acquired guards.
    #[inline]
    pub fn total(&self) -> usize {
        self.0.count.get()
    }
}

struct CounterInner {
    count: Cell<usize>,
    capacity: usize,
    task: LocalWaker,
}

impl CounterInner {
    fn inc(&self) {
        self.count.set(self.count.get() + 1);
    }

    fn dec(&self) {
        let num = self.count.get();
        self.count.set(num - 1);
        if num == self.capacity {
            self.task.wake();
        }
    }

    fn available(&self, cx: &mut task::Context<'_>) -> bool {
        if self.count.get() < self.capacity {
            true
        } else {
            self.task.register(cx.waker());
            false
        }
    }
}

impl fmt::Debug for CounterInner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Counter")
            .field("count", &self.count.get())
            .field("capacity", &self.capacity)
            .field("task", &self.task)
            .finish()
    }
}

/// An RAII structure that keeps the underlying counter incremented until this guard is dropped.
#[derive(Debug)]
pub struct CounterGuard(Rc<CounterInner>);

impl CounterGuard {
    fn new(inner: Rc<CounterInner>) -> Self {
        inner.inc();
        CounterGuard(inner)
    }
}

impl Unpin for CounterGuard {}

impl Drop for CounterGuard {
    fn drop(&mut self) {
        self.0.dec();
    }
}
