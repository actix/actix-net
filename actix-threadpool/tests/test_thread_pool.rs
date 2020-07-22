#[actix_rt::test]
async fn async_work() {
    use core::sync::atomic::{AtomicUsize, Ordering};
    use core::time::Duration;

    use std::sync::Arc;

    let counter = Arc::new(AtomicUsize::new(0));

    for _ in 0..1024 {
        let counter = counter.clone();
        let _ = actix_threadpool::run(move || {
            counter.fetch_add(1, Ordering::Release);
            std::thread::sleep(Duration::from_millis(1));
            Ok::<(), ()>(())
        })
        .await;
    }

    assert_eq!(1024, counter.load(Ordering::Acquire));
}
