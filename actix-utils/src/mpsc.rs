//! A multi-producer, single-consumer, futures-aware, FIFO queue.
use std::any::Any;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::error::Error;
use std::fmt;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

use futures_sink::Sink;
use futures_util::stream::Stream;

use crate::task::LocalWaker;

/// Creates a unbounded in-memory channel with buffered storage.
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
            return Err(SendError(item)); // receiver was dropped
        };
        shared.buffer.push_back(item);
        shared.blocked_recv.wake();
        Ok(())
    }

    /// Closes the sender half
    ///
    /// This prevents any further messages from being sent on the channel while
    /// still enabling the receiver to drain messages that are buffered.
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
/// This is created by the `channel` function.
#[derive(Debug)]
pub struct Receiver<T> {
    shared: Rc<RefCell<Shared<T>>>,
}

impl<T> Receiver<T> {
    /// Create Sender
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
            // All senders have been dropped, so drain the buffer and end the
            // stream.
            Poll::Ready(shared.buffer.pop_front())
        } else if let Some(msg) = shared.buffer.pop_front() {
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

/// Error type for sending, used when the receiving end of a channel is
/// dropped
pub struct SendError<T>(T);

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

impl<T: Any> Error for SendError<T> {
    fn description(&self) -> &str {
        "send failed because receiver is gone"
    }
}

impl<T> SendError<T> {
    /// Returns the message that was attempted to be sent but failed.
    pub fn into_inner(self) -> T {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::future::lazy;
    use futures_util::{stream::Stream, StreamExt};

    #[actix_rt::test]
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
}
