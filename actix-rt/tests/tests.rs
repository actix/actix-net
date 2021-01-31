use std::{
    sync::mpsc::channel,
    thread,
    time::{Duration, Instant},
};

use actix_rt::{Arbiter, System};
use tokio::sync::oneshot;

#[test]
fn await_for_timer() {
    let time = Duration::from_secs(1);
    let instant = Instant::now();
    System::new().block_on(async move {
        tokio::time::sleep(time).await;
    });
    assert!(
        instant.elapsed() >= time,
        "Block on should poll awaited future to completion"
    );
}

#[test]
fn join_another_arbiter() {
    let time = Duration::from_secs(1);
    let instant = Instant::now();
    System::new().block_on(async move {
        let arbiter = Arbiter::new();
        arbiter.spawn(Box::pin(async move {
            tokio::time::sleep(time).await;
            Arbiter::handle().stop();
        }));
        arbiter.join().unwrap();
    });
    assert!(
        instant.elapsed() >= time,
        "Join on another arbiter should complete only when it calls stop"
    );

    let instant = Instant::now();
    System::new().block_on(async move {
        let arbiter = Arbiter::new();
        arbiter.spawn_fn(move || {
            actix_rt::spawn(async move {
                tokio::time::sleep(time).await;
                Arbiter::handle().stop();
            });
        });
        arbiter.join().unwrap();
    });
    assert!(
        instant.elapsed() >= time,
        "Join on an arbiter that has used actix_rt::spawn should wait for said future"
    );

    let instant = Instant::now();
    System::new().block_on(async move {
        let arbiter = Arbiter::new();
        arbiter.spawn(Box::pin(async move {
            tokio::time::sleep(time).await;
            Arbiter::handle().stop();
        }));
        arbiter.stop();
        arbiter.join().unwrap();
    });
    assert!(
        instant.elapsed() < time,
        "Premature stop of arbiter should conclude regardless of it's current state"
    );
}

#[test]
fn non_static_block_on() {
    let string = String::from("test_str");
    let string = string.as_str();

    let sys = System::new();

    sys.block_on(async {
        actix_rt::time::sleep(Duration::from_millis(1)).await;
        assert_eq!("test_str", string);
    });

    let rt = actix_rt::Runtime::new().unwrap();

    rt.block_on(async {
        actix_rt::time::sleep(Duration::from_millis(1)).await;
        assert_eq!("test_str", string);
    });
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
fn arbiter_spawn_fn_runs() {
    let _ = System::new();

    let (tx, rx) = channel::<u32>();

    let arbiter = Arbiter::new();
    arbiter.spawn_fn(move || tx.send(42).unwrap());

    let num = rx.recv().unwrap();
    assert_eq!(num, 42);

    arbiter.stop();
    arbiter.join().unwrap();
}

#[test]
fn arbiter_drop_no_panic_fn() {
    let _ = System::new();

    let arbiter = Arbiter::new();
    arbiter.spawn_fn(|| panic!("test"));

    arbiter.stop();
    arbiter.join().unwrap();
}

#[test]
fn arbiter_drop_no_panic_fut() {
    let _ = System::new();

    let arbiter = Arbiter::new();
    arbiter.spawn(async { panic!("test") });

    arbiter.stop();
    arbiter.join().unwrap();
}

#[test]
fn arbiter_item_storage() {
    let _ = System::new();

    let arbiter = Arbiter::new();

    assert!(!Arbiter::contains_item::<u32>());
    Arbiter::set_item(42u32);
    assert!(Arbiter::contains_item::<u32>());

    Arbiter::get_item(|&item: &u32| assert_eq!(item, 42));
    Arbiter::get_mut_item(|&mut item: &mut u32| assert_eq!(item, 42));

    let thread = thread::spawn(move || {
        Arbiter::get_item(|&_item: &u32| unreachable!("u32 not in this thread"));
    })
    .join();
    assert!(thread.is_err());

    let thread = thread::spawn(move || {
        Arbiter::get_mut_item(|&mut _item: &mut i8| unreachable!("i8 not in this thread"));
    })
    .join();
    assert!(thread.is_err());

    arbiter.stop();
    arbiter.join().unwrap();
}

#[test]
#[should_panic]
fn no_system_current_panic() {
    System::current();
}

#[test]
#[should_panic]
fn no_system_arbiter_new_panic() {
    Arbiter::new();
}

#[test]
fn system_arbiter_spawn() {
    let runner = System::new();

    let (tx, rx) = oneshot::channel();
    let sys = System::current();

    thread::spawn(|| {
        // this thread will have no arbiter in it's thread local so call will panic
        Arbiter::handle();
    })
    .join()
    .unwrap_err();

    let thread = thread::spawn(|| {
        // this thread will have no arbiter in it's thread local so use the system handle instead
        System::set_current(sys);
        let sys = System::current();

        let wrk = sys.arbiter();
        wrk.spawn(async move {
            tx.send(42u32).unwrap();
            System::current().stop();
        });
    });

    assert_eq!(runner.block_on(rx).unwrap(), 42);
    thread.join().unwrap();
}
