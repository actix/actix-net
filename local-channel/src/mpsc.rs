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
        self.shared.borrow_mut().has_receiver = false;
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
        let mut shared = self.shared.borrow_mut();

        if Rc::strong_count(&self.shared) == 1 {
            // All senders have been dropped, so drain the buffer and end the stream.
            return Poll::Ready(shared.buffer.pop_front());
        }

        if let Some(msg) = shared.buffer.pop_front() {
            Poll::Ready(Some(msg))
        } else {
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
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.debug_tuple("SendError").field(&"...").finish()
    }
}

impl<T> fmt::Display for SendError<T> {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(fmt, "send failed because receiver is gone")
    }
}

impl<T> Error for SendError<T> {}

#[cfg(test)]
mod tests {
    use futures_util::{future::lazy, StreamExt as _};

    use super::*;

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
}
