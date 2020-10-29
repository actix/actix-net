use actix_rt::ExecFactory;
use std::time::{Duration, Instant};
use tokio::macros::support::Future;

#[test]
fn await_for_timer() {
    let time = Duration::from_secs(2);
    let instant = Instant::now();

    actix_rt::System::new("test_wait_timer").block_on(async move {
        let arbiter = actix_rt::Arbiter::new();
        arbiter.send(Box::pin(async move {
            tokio::time::delay_for(time).await;
            actix_rt::Arbiter::current().stop();
        }));
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

struct TokioCompatExec;

impl ExecFactory for TokioCompatExec {
    type Executor = tokio_compat::runtime::current_thread::Runtime;
    type Sleep = tokio::time::Delay;

    fn build() -> std::io::Result<Self::Executor> {
        let rt = tokio_compat::runtime::current_thread::Runtime::new()?;

        Ok(rt)
    }

    fn block_on<F: Future>(exec: &mut Self::Executor, f: F) -> <F as Future>::Output {
        exec.block_on_std(f)
    }

    fn spawn<F: Future<Output = ()> + 'static>(f: F) {
        tokio_compat::runtime::current_thread::TaskExecutor::current()
            .spawn_local_std(f)
            .unwrap();
    }

    fn spawn_ref<F: Future<Output = ()> + 'static>(exec: &mut Self::Executor, f: F) {
        exec.spawn_std(f);
    }

    fn sleep(dur: Duration) -> Self::Sleep {
        tokio::time::delay_for(dur)
    }
}

#[test]
fn tokio_compat() -> std::io::Result<()> {
    // manually construct a compat executor.
    let rt = TokioCompatExec::build()?;

    // do some work with rt and pass it to builder
    actix_rt::System::builder::<TokioCompatExec>()
        .create_with_runtime(rt, || {})
        .block_on(async {
            let (tx, rx) = tokio::sync::oneshot::channel();
            tokio_01::spawn(futures_01::lazy(|| {
                tx.send(251).unwrap();
                Ok(())
            }));

            assert_eq!(251, rx.await.unwrap());
        });

    // let the system construct the executor and block on it directly.
    actix_rt::System::new_with::<TokioCompatExec, _>("compat").block_on(async {
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            tokio::time::delay_for(Duration::from_secs(1)).await;
            tx.send(996).unwrap();
        });
        assert_eq!(996, rx.await.unwrap());
    });

    Ok(())
}
