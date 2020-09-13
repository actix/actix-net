use std::time::{Duration, Instant};

#[test]
fn start_and_stop() {
    actix_rt::System::new("start_and_stop").block_on(async move {
        assert!(
            actix_rt::Arbiter::is_running(),
            "System doesn't seem to have started"
        );
    });
    assert!(
        !actix_rt::Arbiter::is_running(),
        "System doesn't seem to have stopped"
    );
}

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

#[test]
fn join_current_arbiter() {
    let time = Duration::from_secs(2);

    let instant = Instant::now();
    actix_rt::System::new("test_join_current_arbiter").block_on(async move {
        actix_rt::spawn(async move {
            tokio::time::delay_for(time).await;
            actix_rt::Arbiter::current().stop();
        });
        actix_rt::Arbiter::local_join().await;
    });
    assert!(
        instant.elapsed() >= time,
        "Join on current arbiter should wait for all spawned futures"
    );

    let large_timer = Duration::from_secs(20);
    let instant = Instant::now();
    actix_rt::System::new("test_join_current_arbiter").block_on(async move {
        actix_rt::spawn(async move {
            tokio::time::delay_for(time).await;
            actix_rt::Arbiter::current().stop();
        });
        let f = actix_rt::Arbiter::local_join();
        actix_rt::spawn(async move {
            tokio::time::delay_for(large_timer).await;
            actix_rt::Arbiter::current().stop();
        });
        f.await;
    });
    assert!(
        instant.elapsed() < large_timer,
        "local_join should await only for the already spawned futures"
    );
}

#[test]
#[cfg(all(feature = "tokio-compat-executor", not(feature = "tokio-executor")))]
fn tokio_compat_timer() {
    use tokio_compat::prelude::*;

    let time = Duration::from_secs(2);
    let instant = Instant::now();
    actix_rt::System::new("test_wait_timer").block_on(async move {
        // Spawn a `std::Future`.
        tokio::time::delay_for(time).await;
        let when = Instant::now() + time;
        // Spawn a `futures01::Future`.
        tokio01::timer::Delay::new(when)
            // convert the delay future into a `std::future` that we can `await`.
            .compat()
            .await
            .expect("tokio 0.1 timer should work!");
    });
    assert!(
        instant.elapsed() >= time * 2,
        "Block on should poll awaited future to completion"
    );
}

#[test]
#[cfg(all(feature = "tokio-compat-executor", not(feature = "tokio-executor")))]
fn tokio_compat_spawn() {
    actix_rt::System::new("test_wait_timer").block_on(async move {
        // Spawning with tokio 0.1 works on the main thread
        tokio01::spawn(futures01::lazy(|| futures01::future::ok(())));

        // Spawning with tokio 0.2 works of course
        tokio::spawn(async {}).await.unwrap();
    });
}
