use std::time::{Duration, Instant};

use actix_rt::{Arbiter, System};
use futures_util::future::lazy;
use tokio::task::LocalSet;

#[test]
fn await_for_timer() {
    let time = Duration::from_secs(1);
    let instant = Instant::now();
    actix_rt::System::new("test_wait_timer").block_on(async move {
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
    actix_rt::System::new("test_join_another_arbiter").block_on(async move {
        let mut arbiter = actix_rt::Arbiter::new();
        arbiter.spawn(Box::pin(async move {
            tokio::time::sleep(time).await;
            actix_rt::Arbiter::current().stop();
        }));
        arbiter.join().unwrap();
    });
    assert!(
        instant.elapsed() >= time,
        "Join on another arbiter should complete only when it calls stop"
    );

    let instant = Instant::now();
    actix_rt::System::new("test_join_another_arbiter").block_on(async move {
        let mut arbiter = actix_rt::Arbiter::new();
        arbiter.spawn_fn(move || {
            actix_rt::spawn(async move {
                tokio::time::sleep(time).await;
                actix_rt::Arbiter::current().stop();
            });
        });
        arbiter.join().unwrap();
    });
    assert!(
        instant.elapsed() >= time,
        "Join on a arbiter that has used actix_rt::spawn should wait for said future"
    );

    let instant = Instant::now();
    actix_rt::System::new("test_join_another_arbiter").block_on(async move {
        let mut arbiter = actix_rt::Arbiter::new();
        arbiter.spawn(Box::pin(async move {
            tokio::time::sleep(time).await;
            actix_rt::Arbiter::current().stop();
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
    let str = string.as_str();

    let sys = actix_rt::System::new("borrow some");

    sys.block_on(async {
        actix_rt::time::sleep(Duration::from_millis(1)).await;
        assert_eq!("test_str", str);
    });

    let rt = actix_rt::Runtime::new().unwrap();

    rt.block_on(async {
        actix_rt::time::sleep(Duration::from_millis(1)).await;
        assert_eq!("test_str", str);
    });

    actix_rt::System::run(|| {
        assert_eq!("test_str", str);
        actix_rt::System::current().stop();
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
