//! An example on how to build a multi-thread tokio runtime for Actix System.
//! Then spawn async task that can make use of work stealing of tokio runtime.

use actix_rt::System;

fn main() {
    System::with_tokio_rt(|| {
        // build system with a multi-thread tokio runtime.
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
    })
    .block_on(async_main());
}

// async main function that acts like #[actix_web::main] or #[tokio::main]
async fn async_main() {
    // use System::spawn to get inside the context of multi thread tokio runtime
    let h1 = System::spawn(async {
        println!("thread id is {:?}", std::thread::current().id());
        std::thread::sleep(std::time::Duration::from_secs(2));
    });

    // work stealing occurs for this task spawn
    let h2 = System::spawn(async {
        println!("thread id is {:?}", std::thread::current().id());
    });

    h1.await.unwrap();
    h2.await.unwrap();

    // note actix_rt can not be used in System::spawn
    let res = System::spawn(async {
        let _ = actix_rt::spawn(actix_rt::task::yield_now()).await;
    })
    .await;

    // result would always be JoinError with panic message logged.
    assert!(res.is_err());
}
