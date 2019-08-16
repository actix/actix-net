use std::marker::PhantomData;
use std::rc::Rc;

use actix_service::{IntoService, NewService, Service};
use futures::unsync::mpsc;
use futures::{Async, Future, Poll, Stream};

type Request<T> = Result<<T as IntoStream>::Item, <T as IntoStream>::Error>;

pub trait IntoStream {
    type Item;
    type Error;
    type Stream: Stream<Item = Self::Item, Error = Self::Error>;

    fn into_stream(self) -> Self::Stream;
}

impl<T> IntoStream for T
where
    T: Stream,
{
    type Item = T::Item;
    type Error = T::Error;
    type Stream = T;

    fn into_stream(self) -> Self::Stream {
        self
    }
}

pub struct StreamService<S, T: NewService, E> {
    factory: Rc<T>,
    config: T::Config,
    _t: PhantomData<(S, E)>,
}

impl<S, T, E> Service for StreamService<S, T, E>
where
    S: IntoStream + 'static,
    T: NewService<Request = Request<S>, Response = (), Error = E, InitError = E>,
    T::Future: 'static,
    T::Service: 'static,
    <T::Service as Service>::Future: 'static,
{
    type Request = S;
    type Response = ();
    type Error = E;
    type Future = Box<dyn Future<Item = (), Error = E>>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, req: S) -> Self::Future {
        Box::new(
            self.factory
                .new_service(&self.config)
                .and_then(move |srv| StreamDispatcher::new(req, srv)),
        )
    }
}

pub struct StreamDispatcher<S, T>
where
    S: IntoStream + 'static,
    T: Service<Request = Request<S>, Response = ()> + 'static,
    T::Future: 'static,
{
    stream: S,
    service: T,
    err_rx: mpsc::UnboundedReceiver<T::Error>,
    err_tx: mpsc::UnboundedSender<T::Error>,
}

impl<S, T> StreamDispatcher<S, T>
where
    S: Stream,
    T: Service<Request = Request<S>, Response = ()>,
    T::Future: 'static,
{
    pub fn new<F1, F2>(stream: F1, service: F2) -> Self
    where
        F1: IntoStream<Stream = S, Item = S::Item, Error = S::Error>,
        F2: IntoService<T>,
    {
        let (err_tx, err_rx) = mpsc::unbounded();
        StreamDispatcher {
            err_rx,
            err_tx,
            stream: stream.into_stream(),
            service: service.into_service(),
        }
    }
}

impl<S, T> Future for StreamDispatcher<S, T>
where
    S: Stream,
    T: Service<Request = Request<S>, Response = ()>,
    T::Future: 'static,
{
    type Item = ();
    type Error = T::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Ok(Async::Ready(Some(e))) = self.err_rx.poll() {
            return Err(e);
        }

        loop {
            match self.service.poll_ready()? {
                Async::Ready(_) => match self.stream.poll() {
                    Ok(Async::Ready(Some(item))) => {
                        tokio_current_thread::spawn(StreamDispatcherService {
                            fut: self.service.call(Ok(item)),
                            stop: self.err_tx.clone(),
                        })
                    }
                    Err(err) => tokio_current_thread::spawn(StreamDispatcherService {
                        fut: self.service.call(Err(err)),
                        stop: self.err_tx.clone(),
                    }),
                    Ok(Async::NotReady) => return Ok(Async::NotReady),
                    Ok(Async::Ready(None)) => return Ok(Async::Ready(())),
                },
                Async::NotReady => return Ok(Async::NotReady),
            }
        }
    }
}

struct StreamDispatcherService<F: Future> {
    fut: F,
    stop: mpsc::UnboundedSender<F::Error>,
}

impl<F: Future> Future for StreamDispatcherService<F> {
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.fut.poll() {
            Ok(Async::Ready(_)) => Ok(Async::Ready(())),
            Ok(Async::NotReady) => Ok(Async::NotReady),
            Err(e) => {
                let _ = self.stop.unbounded_send(e);
                Ok(Async::Ready(()))
            }
        }
    }
}
