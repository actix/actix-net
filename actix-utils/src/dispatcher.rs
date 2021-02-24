//! Framed dispatcher service and related utilities.

use core::{
    fmt,
    future::Future,
    mem,
    pin::Pin,
    task::{Context, Poll},
};

use actix_codec::{AsyncRead, AsyncWrite, Decoder, Encoder, Framed};
use actix_service::{IntoService, Service};
use log::debug;
use pin_project_lite::pin_project;
use tokio::sync::mpsc;

/// Framed transport errors.
pub enum DispatcherError<E, U: Encoder<I> + Decoder, I> {
    Service(E),
    Encoder(<U as Encoder<I>>::Error),
    Decoder(<U as Decoder>::Error),
}

impl<E, U: Encoder<I> + Decoder, I> From<E> for DispatcherError<E, U, I> {
    fn from(err: E) -> Self {
        DispatcherError::Service(err)
    }
}

impl<E, U: Encoder<I> + Decoder, I> fmt::Debug for DispatcherError<E, U, I>
where
    E: fmt::Debug,
    <U as Encoder<I>>::Error: fmt::Debug,
    <U as Decoder>::Error: fmt::Debug,
{
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            DispatcherError::Service(ref e) => write!(fmt, "DispatcherError::Service({:?})", e),
            DispatcherError::Encoder(ref e) => write!(fmt, "DispatcherError::Encoder({:?})", e),
            DispatcherError::Decoder(ref e) => write!(fmt, "DispatcherError::Decoder({:?})", e),
        }
    }
}

impl<E, U: Encoder<I> + Decoder, I> fmt::Display for DispatcherError<E, U, I>
where
    E: fmt::Display,
    <U as Encoder<I>>::Error: fmt::Debug,
    <U as Decoder>::Error: fmt::Debug,
{
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            DispatcherError::Service(ref e) => write!(fmt, "{}", e),
            DispatcherError::Encoder(ref e) => write!(fmt, "{:?}", e),
            DispatcherError::Decoder(ref e) => write!(fmt, "{:?}", e),
        }
    }
}

pub enum Message<T> {
    Item(T),
    Close,
}

pin_project! {
    /// Dispatcher is a future that reads frames from Framed object and passes them to the service.
    pub struct Dispatcher<S, T, U, I>
    where
        S: Service<<U as Decoder>::Item, Response = I>,
        S::Error: 'static,
        S::Future: 'static,
        T: AsyncRead,
        T: AsyncWrite,
        U: Encoder<I>,
        U: Decoder,
        I: 'static,
        <U as Encoder<I>>::Error: fmt::Debug,
    {
        service: S,
        state: State<S, U, I>,
        #[pin]
        framed: Framed<T, U>,
        rx: mpsc::UnboundedReceiver<Result<Message<I>, S::Error>>,
        tx: mpsc::UnboundedSender<Result<Message<I>, S::Error>>,
    }
}

enum State<S, U, I>
where
    S: Service<<U as Decoder>::Item>,
    U: Encoder<I> + Decoder,
{
    Processing,
    Error(DispatcherError<S::Error, U, I>),
    FramedError(DispatcherError<S::Error, U, I>),
    FlushAndStop,
    Stopping,
}

impl<S, U, I> State<S, U, I>
where
    S: Service<<U as Decoder>::Item>,
    U: Encoder<I> + Decoder,
{
    fn take_error(&mut self) -> DispatcherError<S::Error, U, I> {
        match mem::replace(self, State::Processing) {
            State::Error(err) => err,
            _ => panic!(),
        }
    }

    fn take_framed_error(&mut self) -> DispatcherError<S::Error, U, I> {
        match mem::replace(self, State::Processing) {
            State::FramedError(err) => err,
            _ => panic!(),
        }
    }
}

impl<S, T, U, I> Dispatcher<S, T, U, I>
where
    S: Service<<U as Decoder>::Item, Response = I>,
    S::Error: 'static,
    S::Future: 'static,
    T: AsyncRead + AsyncWrite,
    U: Decoder + Encoder<I>,
    I: 'static,
    <U as Decoder>::Error: fmt::Debug,
    <U as Encoder<I>>::Error: fmt::Debug,
{
    pub fn new<F>(framed: Framed<T, U>, service: F) -> Self
    where
        F: IntoService<S, <U as Decoder>::Item>,
    {
        let (tx, rx) = mpsc::unbounded_channel();
        Dispatcher {
            framed,
            rx,
            tx,
            service: service.into_service(),
            state: State::Processing,
        }
    }

    /// Get sink
    pub fn tx(&self) -> mpsc::UnboundedSender<Result<Message<I>, S::Error>> {
        self.tx.clone()
    }

    /// Get reference to a service wrapped by `Dispatcher` instance.
    pub fn service(&self) -> &S {
        &self.service
    }

    /// Get mutable reference to a service wrapped by `Dispatcher` instance.
    pub fn service_mut(&mut self) -> &mut S {
        &mut self.service
    }

    /// Get reference to a framed instance wrapped by `Dispatcher`
    /// instance.
    pub fn framed(&self) -> &Framed<T, U> {
        &self.framed
    }

    /// Get mutable reference to a framed instance wrapped by `Dispatcher` instance.
    pub fn framed_mut(&mut self) -> &mut Framed<T, U> {
        &mut self.framed
    }

    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> bool
    where
        S: Service<<U as Decoder>::Item, Response = I>,
        S::Error: 'static,
        S::Future: 'static,
        T: AsyncRead + AsyncWrite,
        U: Decoder + Encoder<I>,
        I: 'static,
        <U as Encoder<I>>::Error: fmt::Debug,
    {
        loop {
            let this = self.as_mut().project();
            match this.service.poll_ready(cx) {
                Poll::Ready(Ok(_)) => {
                    let item = match this.framed.next_item(cx) {
                        Poll::Ready(Some(Ok(el))) => el,
                        Poll::Ready(Some(Err(err))) => {
                            *this.state = State::FramedError(DispatcherError::Decoder(err));
                            return true;
                        }
                        Poll::Pending => return false,
                        Poll::Ready(None) => {
                            *this.state = State::Stopping;
                            return true;
                        }
                    };

                    let tx = this.tx.clone();
                    let fut = this.service.call(item);
                    actix_rt::spawn(async move {
                        let item = fut.await;
                        let _ = tx.send(item.map(Message::Item));
                    });
                }
                Poll::Pending => return false,
                Poll::Ready(Err(err)) => {
                    *this.state = State::Error(DispatcherError::Service(err));
                    return true;
                }
            }
        }
    }

    /// write to framed object
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> bool
    where
        S: Service<<U as Decoder>::Item, Response = I>,
        S::Error: 'static,
        S::Future: 'static,
        T: AsyncRead + AsyncWrite,
        U: Decoder + Encoder<I>,
        I: 'static,
        <U as Encoder<I>>::Error: fmt::Debug,
    {
        loop {
            let mut this = self.as_mut().project();
            while !this.framed.is_write_buf_full() {
                match this.rx.poll_recv(cx) {
                    Poll::Ready(Some(Ok(Message::Item(msg)))) => {
                        if let Err(err) = this.framed.as_mut().write(msg) {
                            *this.state = State::FramedError(DispatcherError::Encoder(err));
                            return true;
                        }
                    }
                    Poll::Ready(Some(Ok(Message::Close))) => {
                        *this.state = State::FlushAndStop;
                        return true;
                    }
                    Poll::Ready(Some(Err(err))) => {
                        *this.state = State::Error(DispatcherError::Service(err));
                        return true;
                    }
                    Poll::Ready(None) | Poll::Pending => break,
                }
            }

            if !this.framed.is_write_buf_empty() {
                match this.framed.poll_flush(cx) {
                    Poll::Pending => break,
                    Poll::Ready(Ok(_)) => (),
                    Poll::Ready(Err(err)) => {
                        debug!("Error sending data: {:?}", err);
                        *this.state = State::FramedError(DispatcherError::Encoder(err));
                        return true;
                    }
                }
            } else {
                break;
            }
        }

        false
    }
}

impl<S, T, U, I> Future for Dispatcher<S, T, U, I>
where
    S: Service<<U as Decoder>::Item, Response = I>,
    S::Error: 'static,
    S::Future: 'static,
    T: AsyncRead + AsyncWrite,
    U: Decoder + Encoder<I>,
    I: 'static,
    <U as Encoder<I>>::Error: fmt::Debug,
    <U as Decoder>::Error: fmt::Debug,
{
    type Output = Result<(), DispatcherError<S::Error, U, I>>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.as_mut().project();

        match this.state {
            State::Processing => {
                if self.as_mut().poll_read(cx) || self.as_mut().poll_write(cx) {
                    self.poll(cx)
                } else {
                    Poll::Pending
                }
            }

            State::Error(_) => {
                // flush write buffer
                if !this.framed.is_write_buf_empty() && this.framed.poll_flush(cx).is_pending()
                {
                    return Poll::Pending;
                }

                Poll::Ready(Err(this.state.take_error()))
            }

            State::FlushAndStop => {
                if !this.framed.is_write_buf_empty() {
                    this.framed.poll_flush(cx).map(|res| {
                        if let Err(err) = res {
                            debug!("Error sending data: {:?}", err);
                        }

                        Ok(())
                    })
                } else {
                    Poll::Ready(Ok(()))
                }
            }

            State::FramedError(_) => Poll::Ready(Err(this.state.take_framed_error())),
            State::Stopping => Poll::Ready(Ok(())),
        }
    }
}
