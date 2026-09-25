//! A non-thread-safe multi-producer, single-consumer, futures-aware, FIFO queue.

use alloc::{collections::VecDeque, rc::Rc};
use core::{
    cell::RefCell,
    fmt,
    future::poll_fn,
    pin::Pin,
    task::{Context, Poll},
};
use std::error::Error;

use futures_core::stream::Stream;
use futures_sink::Sink;
use local_waker::LocalWaker;

/// Creates a unbounded in-memory channel with buffered storage.
///
/// [Sender]s and [Receiver]s are `!Send`.
pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
    let shared = Rc::new(RefCell::new(Shared {
        has_receiver: true,
        buffer: VecDeque::new(),
        blocked_recv: LocalWaker::new(),
    }));

    let sender = Sender {
        shared: shared.clone(),
    };

    let receiver = Receiver { shared };

    (sender, receiver)
}

#[derive(Debug)]
struct Shared<T> {
    buffer: VecDeque<T>,
    blocked_recv: LocalWaker,
    has_receiver: bool,
}

/// The transmission end of a channel.
///
/// This is created by the `channel` function.
#[derive(Debug)]
pub struct Sender<T> {
    shared: Rc<RefCell<Shared<T>>>,
}

impl<T> Unpin for Sender<T> {}

impl<T> Sender<T> {
    /// Sends the provided message along this channel.
    pub fn send(&self, item: T) -> Result<(), SendError<T>> {
        let mut shared = self.shared.borrow_mut();

        if !shared.has_receiver {
            // receiver was dropped
            return Err(SendError(item));
        };

        shared.buffer.push_back(item);
        shared.blocked_recv.wake();

        Ok(())
    }

    /// Closes the sender half.
    ///
    /// This prevents any further messages from being sent on the channel, by any sender, while
    /// still enabling the receiver to drain messages that are already buffered.
    pub fn close(&mut self) {
        let mut shared = self.shared.borrow_mut();
        shared.has_receiver = false;
        shared.blocked_recv.wake();
    }
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        Sender {
            shared: self.shared.clone(),
        }
    }
}

impl<T> Sink<T> for Sender<T> {
    type Error = SendError<T>;

    fn poll_ready(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn start_send(self: Pin<&mut Self>, item: T) -> Result<(), SendError<T>> {
        self.send(item)
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), SendError<T>>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.get_mut().close();
        Poll::Ready(Ok(()))
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        let count = Rc::strong_count(&self.shared);
        let shared = self.shared.borrow_mut();

        // check is last sender is about to drop
        if shared.has_receiver && count == 2 {
            // Wake up receiver as its stream has ended
            shared.blocked_recv.wake();
        }
    }
}

/// The receiving end of a channel which implements the `Stream` trait.
///
/// This is created by the [`channel`] function.
#[derive(Debug)]
pub struct Receiver<T> {
    shared: Rc<RefCell<Shared<T>>>,
}

impl<T> Receiver<T> {
    /// Receive the next value.
    ///
    /// Returns `None` if the channel is empty and has been [closed](Sender::close) explicitly or
    /// when all senders have been dropped and, therefore, no more values can ever be sent though
    /// this channel.
    pub async fn recv(&mut self) -> Option<T> {
        let mut this = Pin::new(self);
        poll_fn(|cx| this.as_mut().poll_next(cx)).await
    }

    /// Create an associated [Sender].
    pub fn sender(&self) -> Sender<T> {
        Sender {
            shared: self.shared.clone(),
        }
    }
}

impl<T> Unpin for Receiver<T> {}

impl<T> Stream for Receiver<T> {
    type Item = T;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let all_senders_dropped = Rc::strong_count(&self.shared) == 1;

        let mut shared = self.shared.borrow_mut();

        if let Some(msg) = shared.buffer.pop_front() {
            // Always drain buffered messages before ending the stream.
            Poll::Ready(Some(msg))
        } else if !shared.has_receiver || all_senders_dropped {
            // The buffer is empty and no more messages can arrive.
            Poll::Ready(None)
        } else {
            // Buffer is empty, but channel is open and senders exist.
            shared.blocked_recv.register(cx.waker());
            Poll::Pending
        }
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        let mut shared = self.shared.borrow_mut();
        shared.buffer.clear();
        shared.has_receiver = false;
    }
}

/// Error returned when attempting to send after the channels' [Receiver] is dropped or closed.
///
/// Allows access to message that failed to send with [`into_inner`](Self::into_inner).
pub struct SendError<T>(pub T);

impl<T> SendError<T> {
    /// Returns the message that was attempted to be sent but failed.
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> fmt::Debug for SendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("SendError").finish_non_exhaustive()
    }
}

impl<T> fmt::Display for SendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Send failed because receiver has been closed or dropped")
    }
}

impl<T> Error for SendError<T> {}

#[cfg(test)]
mod tests {
    use std::{pin::pin, task::Waker};

    use futures_util::{future::lazy, StreamExt as _};
    use tokio_test::{assert_pending, assert_ready, assert_ready_eq};

    use super::*;

    static_assertions::assert_not_impl_all!(Sender<()>: Send, Sync);
    static_assertions::assert_not_impl_all!(Receiver<()>: Send, Sync);
    static_assertions::assert_not_impl_all!(Sender<RefCell<()>>: Send, Sync);
    static_assertions::assert_not_impl_all!(Receiver<RefCell<()>>: Send, Sync);

    #[tokio::test]
    async fn test_mpsc() {
        let (tx, mut rx) = channel();
        tx.send("test").unwrap();
        assert_eq!(rx.next().await.unwrap(), "test");

        let tx2 = tx.clone();
        tx2.send("test2").unwrap();
        assert_eq!(rx.next().await.unwrap(), "test2");

        assert_eq!(
            lazy(|cx| Pin::new(&mut rx).poll_next(cx)).await,
            Poll::Pending
        );
        drop(tx2);
        assert_eq!(
            lazy(|cx| Pin::new(&mut rx).poll_next(cx)).await,
            Poll::Pending
        );
        drop(tx);
        assert_eq!(rx.next().await, None);

        let (tx, rx) = channel();
        tx.send("test").unwrap();
        drop(rx);
        assert!(tx.send("test").is_err());

        let (mut tx, _) = channel();
        let tx2 = tx.clone();
        tx.close();
        assert!(tx.send("test").is_err());
        assert!(tx2.send("test").is_err());
    }

    #[tokio::test]
    async fn test_recv() {
        let (tx, mut rx) = channel();
        tx.send("test").unwrap();
        assert_eq!(rx.recv().await.unwrap(), "test");
        drop(tx);

        let (tx, mut rx) = channel();
        tx.send("test").unwrap();
        assert_eq!(rx.recv().await.unwrap(), "test");
        drop(tx);
        assert!(rx.recv().await.is_none());
    }

    #[tokio::test]
    async fn sink_close_stops_sends_and_ends_receiver_after_buffered_messages() {
        let (sender, receiver) = channel();
        sender.send(1).unwrap();
        sender.send(2).unwrap();

        let mut sender = pin!(sender);
        let mut receiver = pin!(receiver);
        let mut cx = Context::from_waker(Waker::noop());

        assert_ready!(sender.as_mut().poll_close(&mut cx)).unwrap();
        assert!(sender.send(3).is_err());
        assert_ready_eq!(receiver.as_mut().poll_next(&mut cx), Some(1));
        assert_ready_eq!(receiver.as_mut().poll_next(&mut cx), Some(2));
        assert_ready_eq!(receiver.as_mut().poll_next(&mut cx), None);
    }

    #[tokio::test]
    async fn close_drains_fifo_and_rejects_sends_with_cloned_senders_alive() {
        let (mut sender, receiver) = channel();
        let sender_clone = sender.clone();

        sender.send(1).unwrap();
        sender_clone.send(2).unwrap();
        sender.close();

        assert_eq!(sender.send(3).unwrap_err().into_inner(), 3);
        assert_eq!(sender_clone.send(4).unwrap_err().into_inner(), 4);

        let mut receiver = pin!(receiver);
        let mut cx = Context::from_waker(Waker::noop());

        assert_ready_eq!(receiver.as_mut().poll_next(&mut cx), Some(1));
        assert_ready_eq!(receiver.as_mut().poll_next(&mut cx), Some(2));
        assert_ready_eq!(receiver.as_mut().poll_next(&mut cx), None);
    }

    #[tokio::test]
    async fn close_wakes_pending_receiver_before_senders_are_dropped() {
        use std::{
            sync::{
                atomic::{AtomicBool, Ordering},
                Arc,
            },
            task::Wake,
        };

        struct WakeFlag(AtomicBool);

        impl Wake for WakeFlag {
            fn wake(self: Arc<Self>) {
                self.0.store(true, Ordering::SeqCst);
            }

            fn wake_by_ref(self: &Arc<Self>) {
                self.0.store(true, Ordering::SeqCst);
            }
        }

        let (mut sender, receiver) = channel::<u8>();
        let sender_clone = sender.clone();
        let wake_flag = Arc::new(WakeFlag(AtomicBool::new(false)));
        let waker = Waker::from(wake_flag.clone());
        let mut cx = Context::from_waker(&waker);
        let mut receiver = pin!(receiver);

        assert_pending!(receiver.as_mut().poll_next(&mut cx));
        assert!(!wake_flag.0.load(Ordering::SeqCst));

        sender.close();
        assert!(wake_flag.0.load(Ordering::SeqCst));

        drop(sender);
        drop(sender_clone);
        assert_ready_eq!(receiver.as_mut().poll_next(&mut cx), None);
    }
}
