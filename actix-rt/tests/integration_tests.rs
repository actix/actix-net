use std::time::{Duration, Instant};

#[test]
fn await_for_timer() {
    let time = Duration::from_secs(2);
    let instant = Instant::now();
    actix_rt::System::new("test_wait_timer").block_on(async move {
        tokio::time::delay_for(time).await;
    });
    assert!(
        instant.elapsed() >= time,
        "Block on should poll awaited future to completion"
    );
}

#[test]
fn join_another_arbiter() {
    let time = Duration::from_secs(2);
    let instant = Instant::now();
    actix_rt::System::new("test_join_another_arbiter").block_on(async move {
        let mut arbiter = actix_rt::Arbiter::new();
        arbiter.send(Box::pin(async move {
            tokio::time::delay_for(time).await;
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
        arbiter.exec_fn(move || {
            actix_rt::spawn(async move {
                tokio::time::delay_for(time).await;
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
        arbiter.send(Box::pin(async move {
            tokio::time::delay_for(time).await;
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

// #[test]
// fn join_current_arbiter() {
//     let time = Duration::from_secs(2);
//
//     let instant = Instant::now();
//     actix_rt::System::new("test_join_current_arbiter").block_on(async move {
//         actix_rt::spawn(async move {
//             tokio::time::delay_for(time).await;
//             actix_rt::Arbiter::current().stop();
//         });
//         actix_rt::Arbiter::local_join().await;
//     });
//     assert!(
//         instant.elapsed() >= time,
//         "Join on current arbiter should wait for all spawned futures"
//     );
//
//     let large_timer = Duration::from_secs(20);
//     let instant = Instant::now();
//     actix_rt::System::new("test_join_current_arbiter").block_on(async move {
//         actix_rt::spawn(async move {
//             tokio::time::delay_for(time).await;
//             actix_rt::Arbiter::current().stop();
//         });
//         let f = actix_rt::Arbiter::local_join();
//         actix_rt::spawn(async move {
//             tokio::time::delay_for(large_timer).await;
//             actix_rt::Arbiter::current().stop();
//         });
//         f.await;
//     });
//     assert!(
//         instant.elapsed() < large_timer,
//         "local_join should await only for the already spawned futures"
//     );
// }

#[test]
fn non_static_block_on() {
    let string = String::from("test_str");
    let str = string.as_str();

    let mut sys = actix_rt::System::new("borrow some");

    sys.block_on(async {
        actix_rt::time::delay_for(Duration::from_millis(1)).await;
        assert_eq!("test_str", str);
    });

    let mut rt = actix_rt::Runtime::new().unwrap();

    rt.block_on(async {
        actix_rt::time::delay_for(Duration::from_millis(1)).await;
        assert_eq!("test_str", str);
    });

    actix_rt::System::run(|| {
        assert_eq!("test_str", str);
        actix_rt::System::current().stop();
    })
    .unwrap();
}
