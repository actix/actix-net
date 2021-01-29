use std::{
    thread,
    time::{Duration, Instant},
};

use actix_rt::{System, Worker};

#[test]
fn await_for_timer() {
    let time = Duration::from_secs(1);
    let instant = Instant::now();
    System::new("test_wait_timer").block_on(async move {
        tokio::time::sleep(time).await;
    });
    assert!(
        instant.elapsed() >= time,
        "Block on should poll awaited future to completion"
    );
}

#[test]
fn join_another_worker() {
    let time = Duration::from_secs(1);
    let instant = Instant::now();
    System::new("test_join_another_worker").block_on(async move {
        let mut worker = Worker::new();
        worker.spawn(Box::pin(async move {
            tokio::time::sleep(time).await;
            Worker::current().stop();
        }));
        worker.join().unwrap();
    });
    assert!(
        instant.elapsed() >= time,
        "Join on another worker should complete only when it calls stop"
    );

    let instant = Instant::now();
    System::new("test_join_another_worker").block_on(async move {
        let mut worker = Worker::new();
        worker.spawn_fn(move || {
            actix_rt::spawn(async move {
                tokio::time::sleep(time).await;
                Worker::current().stop();
            });
        });
        worker.join().unwrap();
    });
    assert!(
        instant.elapsed() >= time,
        "Join on a worker that has used actix_rt::spawn should wait for said future"
    );

    let instant = Instant::now();
    System::new("test_join_another_worker").block_on(async move {
        let mut worker = Worker::new();
        worker.spawn(Box::pin(async move {
            tokio::time::sleep(time).await;
            Worker::current().stop();
        }));
        worker.stop();
        worker.join().unwrap();
    });
    assert!(
        instant.elapsed() < time,
        "Premature stop of worker should conclude regardless of it's current state"
    );
}

#[test]
fn non_static_block_on() {
    let string = String::from("test_str");
    let str = string.as_str();

    let sys = System::new("borrow some");

    sys.block_on(async {
        actix_rt::time::sleep(Duration::from_millis(1)).await;
        assert_eq!("test_str", str);
    });

    let rt = actix_rt::Runtime::new().unwrap();

    rt.block_on(async {
        actix_rt::time::sleep(Duration::from_millis(1)).await;
        assert_eq!("test_str", str);
    });

    System::run(|| {
        assert_eq!("test_str", str);
        System::current().stop();
    })
    .unwrap();
}

#[test]
fn wait_for_spawns() {
    let rt = actix_rt::Runtime::new().unwrap();

    let handle = rt.spawn(async {
        println!("running on the runtime");
        // assertion panic is caught at task boundary
        assert_eq!(1, 2);
    });

    assert!(rt.block_on(handle).is_err());
}

#[test]
#[should_panic]
fn worker_drop_panic_fn() {
    let _ = System::new("test-system");

    let mut worker = Worker::new();
    worker.spawn_fn(|| panic!("test"));

    worker.join().unwrap();
}

#[test]
fn worker_drop_no_panic_fut() {
    use futures_util::future::lazy;

    let _ = System::new("test-system");

    let mut worker = Worker::new();
    worker.spawn(lazy(|_| panic!("test")));

    worker.stop();
    worker.join().unwrap();
}

#[test]
fn worker_item_storage() {
    let _ = System::new("test-system");

    let mut worker = Worker::new();

    assert!(!Worker::contains_item::<u32>());
    Worker::set_item(42u32);
    assert!(Worker::contains_item::<u32>());

    Worker::get_item(|&item: &u32| assert_eq!(item, 42));
    Worker::get_mut_item(|&mut item: &mut u32| assert_eq!(item, 42));

    let thread = thread::spawn(move || {
        Worker::get_item(|&_item: &u32| unreachable!("u32 not in this thread"));
    })
    .join();
    assert!(thread.is_err());

    let thread = thread::spawn(move || {
        Worker::get_mut_item(|&mut _item: &mut i8| unreachable!("i8 not in this thread"));
    })
    .join();
    assert!(thread.is_err());

    worker.stop();
    worker.join().unwrap();
}
