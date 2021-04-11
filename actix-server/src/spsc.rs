use std::{
    sync::Arc,
    task::{Context, Poll},
};

use futures_core::task::__internal::AtomicWaker;
use rtrb::{Consumer, Producer, PushError, RingBuffer};

pub fn channel<T>(cap: usize) -> (Sender<T>, Receiver<T>) {
    let (tx, rx) = RingBuffer::new(cap).split();
    let waker = Arc::new(AtomicWaker::new());
    let sender = Sender {
        tx,
        waker: waker.clone(),
    };

    let receiver = Receiver { rx, waker };

    (sender, receiver)
}

pub struct Sender<T> {
    tx: Producer<T>,
    waker: Arc<AtomicWaker>,
}

impl<T> Sender<T> {
    pub fn send(&mut self, item: T) -> Result<(), T> {
        match self.tx.push(item) {
            Ok(_) => {
                self.waker.wake();
                Ok(())
            }
            Err(PushError::Full(item)) => Err(item),
        }
    }
}

pub struct Receiver<T> {
    rx: Consumer<T>,
    waker: Arc<AtomicWaker>,
}

impl<T> Receiver<T> {
    pub fn poll_recv_unpin(&mut self, cx: &mut Context<'_>) -> Poll<T> {
        match self.rx.pop() {
            Ok(item) => Poll::Ready(item),
            Err(_) => {
                self.waker.register(cx.waker());
                Poll::Pending
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use actix_utils::future::poll_fn;

    #[actix_rt::test]
    async fn test_send() {
        let (mut tx, mut rx) = channel::<usize>(1);

        let res = poll_fn(|cx| Poll::Ready(rx.poll_recv_unpin(cx))).await;
        assert!(res.is_pending());

        tx.send(996).unwrap();

        let res = poll_fn(|cx| rx.poll_recv_unpin(cx)).await;
        assert_eq!(res, 996);
    }
}
