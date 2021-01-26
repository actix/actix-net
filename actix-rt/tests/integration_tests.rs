use std::time::{Duration, Instant};

use futures_util::future::try_join_all;

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
        arbiter.send(Box::pin(async move {
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
        arbiter.exec_fn(move || {
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
        arbiter.send(Box::pin(async move {
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

#[test]
fn run_in_existing_tokio() {
    use actix_rt::System;
    use futures_util::future::try_join_all;
    use tokio::task::LocalSet;

    async fn run_application() {
        let first_task = tokio::spawn(async {
            println!("One task");
            Ok::<(), ()>(())
        });

        let second_task = tokio::spawn(async {
            println!("Another task");
            Ok::<(), ()>(())
        });

        try_join_all(vec![first_task, second_task])
            .await
            .expect("Some of the futures finished unexpectedly");
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();

    let actix_local_set = LocalSet::new();
    let sys = System::run_in_tokio("actix-main-system", &actix_local_set);
    actix_local_set.spawn_local(sys);

    let rest_operations = run_application();
    runtime.block_on(actix_local_set.run_until(rest_operations));
}

async fn run_application() -> usize {
    let first_task = tokio::spawn(async {
        println!("One task");
        Ok::<(), ()>(())
    });

    let second_task = tokio::spawn(async {
        println!("Another task");
        Ok::<(), ()>(())
    });

    let tasks = try_join_all(vec![first_task, second_task])
        .await
        .expect("Some of the futures finished unexpectedly");

    tasks.len()
}

#[test]
fn attack_to_tokio() {
    use actix_rt::System;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();

    let rest_operations = run_application();
    let res = System::attach_to_tokio("actix-main-system", runtime, rest_operations);

    assert_eq!(res, 2);
}

#[tokio::test]
async fn attack_to_tokio_macro() {
    use actix_rt::System;

    let rest_operations = run_application();
    let res = System::attach_to_tokio(
        "actix-main-system",
        tokio::runtime::Runtime::handle(&self),
        rest_operations,
    );

    assert_eq!(res, 2);
}
