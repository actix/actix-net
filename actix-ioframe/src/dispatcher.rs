//! Framed dispatcher service and related utilities
use std::pin::Pin;
use std::task::{Context, Poll};

use actix_codec::{AsyncRead, AsyncWrite, Decoder, Encoder, Framed};
use actix_service::Service;
use actix_utils::mpsc;
use futures::Stream;
use pin_project::pin_project;
use log::debug;

use crate::error::ServiceError;

type Request<U> = <U as Decoder>::Item;
type Response<U> = <U as Encoder>::Item;

/// FramedTransport - is a future that reads frames from Framed object
/// and pass then to the service.
#[pin_project]
pub(crate) struct Dispatcher<S, T, U, Out>
where
    S: Service<Request = Request<U>, Response = Option<Response<U>>>,
    S::Error: 'static,
    S::Future: 'static,
    T: AsyncRead + AsyncWrite,
    U: Encoder + Decoder,
    <U as Encoder>::Item: 'static,
    <U as Encoder>::Error: std::fmt::Debug,
    Out: Stream<Item = <U as Encoder>::Item> + Unpin,
{
    service: S,
    sink: Option<Out>,
    state: FramedState<S, U>,
    #[pin]
    framed: Framed<T, U>,
    rx: mpsc::Receiver<Result<<U as Encoder>::Item, S::Error>>,
}

impl<S, T, U, Out> Dispatcher<S, T, U, Out>
where
    S: Service<Request = Request<U>, Response = Option<Response<U>>>,
    S::Error: 'static,
    S::Future: 'static,
    T: AsyncRead + AsyncWrite,
    U: Decoder + Encoder,
    <U as Encoder>::Item: 'static,
    <U as Encoder>::Error: std::fmt::Debug,
    Out: Stream<Item = <U as Encoder>::Item> + Unpin,
{
    pub(crate) fn new(framed: Framed<T, U>, service: S, sink: Option<Out>) -> Self {
        Dispatcher {
            sink,
            service,
            framed,
            rx: mpsc::channel().1,
            state: FramedState::Processing,
        }
    }
}

enum FramedState<S: Service, U: Encoder + Decoder> {
    Processing,
    Error(ServiceError<S::Error, U>),
    FramedError(ServiceError<S::Error, U>),
    FlushAndStop,
    Stopping,
}

impl<S: Service, U: Encoder + Decoder> FramedState<S, U> {
    fn take_error(&mut self) -> ServiceError<S::Error, U> {
        match std::mem::replace(self, FramedState::Processing) {
            FramedState::Error(err) => err,
            _ => panic!(),
        }
    }

    fn take_framed_error(&mut self) -> ServiceError<S::Error, U> {
        match std::mem::replace(self, FramedState::Processing) {
            FramedState::FramedError(err) => err,
            _ => panic!(),
        }
    }
}

impl<S, T, U, Out> Dispatcher<S, T, U, Out>
where
    S: Service<Request = Request<U>, Response = Option<Response<U>>>,
    S::Error: 'static,
    S::Future: 'static,
    T: AsyncRead + AsyncWrite,
    U: Decoder + Encoder,
    <U as Encoder>::Item: 'static,
    <U as Encoder>::Error: std::fmt::Debug,
    Out: Stream<Item = <U as Encoder>::Item> + Unpin,
{
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> bool {
        loop {
            let this = self.as_mut().project();
            match this.service.poll_ready(cx) {
                Poll::Ready(Ok(_)) => {
                    let item = match this.framed.next_item(cx) {
                        Poll::Ready(Some(Ok(el))) => el,
                        Poll::Ready(Some(Err(err))) => {
                            *this.state = FramedState::FramedError(ServiceError::Decoder(err));
                            return true;
                        }
                        Poll::Pending => return false,
                        Poll::Ready(None) => {
                            log::trace!("Client disconnected");
                            *this.state = FramedState::Stopping;
                            return true;
                        }
                    };

                    let tx = this.rx.sender();
                    let fut = this.service.call(item);
                    actix_rt::spawn(async move {
                        let item = fut.await;
                        let item = match item {
                            Ok(Some(item)) => Ok(item),
                            Ok(None) => return,
                            Err(err) => Err(err),
                        };
                        let _ = tx.send(item);
                    });
                }
                Poll::Pending => return false,
                Poll::Ready(Err(err)) => {
                    *this.state = FramedState::Error(ServiceError::Service(err));
                    return true;
                }
            }
        }
    }

    /// write to framed object
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> bool {
        loop {
            let mut this = self.as_mut().project();
            while !this.framed.is_write_buf_full() {
                match Pin::new(&mut this.rx).poll_next(cx) {
                    Poll::Ready(Some(Ok(msg))) => {
                        if let Err(err) = this.framed.as_mut().write(msg) {
                            *this.state = FramedState::FramedError(ServiceError::Encoder(err));
                            return true;
                        }
                        continue;
                    }
                    Poll::Ready(Some(Err(err))) => {
                        *this.state = FramedState::Error(ServiceError::Service(err));
                        return true;
                    }
                    Poll::Ready(None) | Poll::Pending => (),
                }

                if this.sink.is_some() {
                    match Pin::new(this.sink.as_mut().unwrap()).poll_next(cx) {
                        Poll::Ready(Some(msg)) => {
                            if let Err(err) = this.framed.as_mut().write(msg) {
                                *this.state =
                                    FramedState::FramedError(ServiceError::Encoder(err));
                                return true;
                            }
                            continue;
                        }
                        Poll::Ready(None) => {
                            let _ = this.sink.take();
                            *this.state = FramedState::FlushAndStop;
                            return true;
                        }
                        Poll::Pending => (),
                    }
                }
                break;
            }

            if !this.framed.is_write_buf_empty() {
                match this.framed.as_mut().flush(cx) {
                    Poll::Pending => break,
                    Poll::Ready(Ok(_)) => (),
                    Poll::Ready(Err(err)) => {
                        debug!("Error sending data: {:?}", err);
                        *this.state = FramedState::FramedError(ServiceError::Encoder(err));
                        return true;
                    }
                }
            } else {
                break;
            }
        }
        false
    }

    pub(crate) fn poll(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), ServiceError<S::Error, U>>> {
        let mut this = self.as_mut().project();
        match this.state {
            FramedState::Processing => loop {
                let read = self.as_mut().poll_read(cx);
                let write = self.as_mut().poll_write(cx);
                if read || write {
                    continue;
                } else {
                    return Poll::Pending;
                }
            },
            FramedState::Error(_) => {
                // flush write buffer
                if !this.framed.is_write_buf_empty() {
                    if let Poll::Pending = this.framed.flush(cx) {
                        return Poll::Pending;
                    }
                }
                Poll::Ready(Err(this.state.take_error()))
            }
            FramedState::FlushAndStop => {
                // drain service responses
                match Pin::new(this.rx).poll_next(cx) {
                    Poll::Ready(Some(Ok(msg))) => {
                        if this.framed.as_mut().write(msg).is_err() {
                            return Poll::Ready(Ok(()));
                        }
                    }
                    Poll::Ready(Some(Err(_))) => return Poll::Ready(Ok(())),
                    Poll::Ready(None) | Poll::Pending => (),
                }

                // flush io
                if !this.framed.is_write_buf_empty() {
                    match this.framed.flush(cx) {
                        Poll::Ready(Err(err)) => {
                            debug!("Error sending data: {:?}", err);
                        }
                        Poll::Pending => {
                            return Poll::Pending;
                        }
                        Poll::Ready(_) => (),
                    }
                };
                Poll::Ready(Ok(()))
            }
            FramedState::FramedError(_) => Poll::Ready(Err(this.state.take_framed_error())),
            FramedState::Stopping => Poll::Ready(Ok(())),
        }
    }
}
