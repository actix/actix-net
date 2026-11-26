//! Tests the public `LocalWaker` behavior.

use std::{
    cell::RefCell,
    rc::Rc,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    task::{Context, Wake, Waker},
};

use local_waker::LocalWaker;

struct WakeCount(AtomicUsize);

impl WakeCount {
    fn new(val: usize) -> Arc<Self> {
        Arc::new(Self(AtomicUsize::new(val)))
    }

    fn waker(self: &Arc<Self>) -> Waker {
        Waker::from(self.clone())
    }
}

fn cx(waker: &Waker) -> Context<'_> {
    Context::from_waker(waker)
}

impl Wake for WakeCount {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

#[test]
fn register_replaces_waker_and_wake_clears_it() {
    let local_waker = LocalWaker::new();
    let first_count = WakeCount::new(0);
    let second_count = WakeCount::new(0);
    let first = first_count.waker();
    let second = second_count.waker();

    assert!(!local_waker.register(&mut cx(&first)));
    assert!(local_waker.register(&mut cx(&first)));
    assert!(local_waker.register(&mut cx(&second)));

    local_waker.wake();
    assert_eq!(first_count.0.load(Ordering::Relaxed), 0);
    assert_eq!(second_count.0.load(Ordering::Relaxed), 1);

    local_waker.wake();
    assert_eq!(second_count.0.load(Ordering::Relaxed), 1);
    assert!(local_waker.take().is_none());

    assert!(!local_waker.register(&mut cx(&first)));
    local_waker.wake();
    assert_eq!(first_count.0.load(Ordering::Relaxed), 1);
}

#[test]
fn take_returns_waker_and_clears_registration() {
    let local_waker = LocalWaker::new();
    let count = WakeCount::new(0);
    let waker = count.waker();

    assert!(local_waker.take().is_none());
    local_waker.wake();

    assert!(!local_waker.register(&mut cx(&waker)));
    let taken = local_waker.take().unwrap();
    assert!(local_waker.take().is_none());
    taken.wake();
    assert_eq!(count.0.load(Ordering::Relaxed), 1);

    local_waker.wake();
    assert_eq!(count.0.load(Ordering::Relaxed), 1);

    assert!(!local_waker.register(&mut cx(&waker)));
    local_waker.wake();
    assert_eq!(count.0.load(Ordering::Relaxed), 2);
}

#[test]
fn wake_keeps_a_waker_registered_by_its_callback() {
    thread_local! {
        static ON_WAKE: RefCell<Option<Box<dyn FnOnce()>>> = RefCell::new(None);
    }

    struct ReRegister;

    impl Wake for ReRegister {
        fn wake(self: Arc<Self>) {
            ON_WAKE.with(|slot| {
                let callback = slot.borrow_mut().take();
                callback.unwrap()();
            });
        }
    }

    let local_waker = Rc::new(LocalWaker::new());
    let first = Waker::from(Arc::new(ReRegister));
    let count = WakeCount::new(0);

    let local_waker_for_callback = Rc::clone(&local_waker);
    let second = count.waker();

    ON_WAKE.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(move || {
            assert!(!local_waker_for_callback.register(&mut cx(&second)));
        }));
    });

    assert!(!local_waker.register(&mut cx(&first)));
    local_waker.wake();
    local_waker.wake();

    assert_eq!(count.0.load(Ordering::Relaxed), 1);
    assert!(local_waker.take().is_none());
}
