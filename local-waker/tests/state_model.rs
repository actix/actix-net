//! Checks the public `LocalWaker` state transitions.

use std::{
    cell::RefCell,
    rc::Rc,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    task::{Wake, Waker},
};

use local_waker::LocalWaker;

thread_local! {
    static ON_WAKE: RefCell<Option<Box<dyn FnOnce()>>> = RefCell::new(None);
}

struct WakeCount(AtomicUsize);

impl Wake for WakeCount {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
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

#[test]
fn all_short_operation_sequences_match_model() {
    const OPERATION_COUNT: usize = 4;
    const SEQUENCE_LENGTH: usize = if cfg!(miri) { 6 } else { 8 };

    for mut sequence in 0..OPERATION_COUNT.pow(SEQUENCE_LENGTH as u32) {
        let local_waker = LocalWaker::new();
        let counts = [
            Arc::new(WakeCount(AtomicUsize::new(0))),
            Arc::new(WakeCount(AtomicUsize::new(0))),
        ];
        let wakers = [
            Waker::from(counts[0].clone()),
            Waker::from(counts[1].clone()),
        ];
        let mut registered = None;
        let mut expected_wakes = [0, 0];

        for _ in 0..SEQUENCE_LENGTH {
            match sequence % OPERATION_COUNT {
                0 | 1 => {
                    let index = sequence % OPERATION_COUNT;
                    assert_eq!(local_waker.register(&wakers[index]), registered.is_some());
                    registered = Some(index);
                }
                2 => {
                    local_waker.wake();
                    if let Some(index) = registered.take() {
                        expected_wakes[index] += 1;
                    }
                }
                3 => {
                    let taken = local_waker.take();
                    assert_eq!(taken.is_some(), registered.is_some());
                    if let Some(index) = registered.take() {
                        taken.unwrap().wake();
                        expected_wakes[index] += 1;
                    }
                }
                _ => unreachable!(),
            }

            for (count, expected) in counts.iter().zip(expected_wakes) {
                assert_eq!(count.0.load(Ordering::Relaxed), expected);
            }

            sequence /= OPERATION_COUNT;
        }

        let taken = local_waker.take();
        assert_eq!(taken.is_some(), registered.is_some());
        if let Some(index) = registered {
            taken.unwrap().wake();
            expected_wakes[index] += 1;
        }

        for (count, expected) in counts.iter().zip(expected_wakes) {
            assert_eq!(count.0.load(Ordering::Relaxed), expected);
        }
    }
}

#[test]
fn wake_keeps_a_waker_registered_by_its_callback() {
    let local_waker = Rc::new(LocalWaker::new());
    let first = Waker::from(Arc::new(ReRegister));
    let count = Arc::new(WakeCount(AtomicUsize::new(0)));
    let second = Waker::from(count.clone());

    let local_waker_for_callback = local_waker.clone();
    ON_WAKE.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(move || {
            assert!(!local_waker_for_callback.register(&second));
        }));
    });

    assert!(!local_waker.register(&first));
    local_waker.wake();
    local_waker.wake();

    assert_eq!(count.0.load(Ordering::Relaxed), 1);
    assert!(local_waker.take().is_none());
}
