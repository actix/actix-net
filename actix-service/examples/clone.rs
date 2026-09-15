#![allow(missing_docs)]

use std::{future::Future, sync::mpsc, time::Duration};

async fn oracle<F, Fut>(f: F) -> (u32, u32)
where
    F: FnOnce() -> Fut + Clone + Send + 'static,
    Fut: Future<Output = u32> + 'static,
{
    let f1 = tokio::task::spawn_local(f.clone()());
    let f2 = tokio::task::spawn_local(f());

    (f1.await.unwrap(), f2.await.unwrap())
}

#[tokio::main(flavor = "local")]
async fn main() {
    let (tx, rx) = mpsc::channel();

    let (r1, r2) = oracle({
        let tx = tx.clone();

        || async move {
            tx.send(()).unwrap();
            4 * 4
        }
    })
    .await;

    assert_eq!(r1, r2);

    tx.send(()).unwrap();

    rx.recv_timeout(Duration::from_millis(100)).unwrap();
    rx.recv_timeout(Duration::from_millis(100)).unwrap();

    println!("Done! Both cloned calls returned {r1}.");
}
