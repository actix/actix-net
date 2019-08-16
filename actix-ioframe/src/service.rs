use std::marker::PhantomData;
use std::rc::Rc;

use actix_codec::{AsyncRead, AsyncWrite, Decoder, Encoder};
use actix_service::{IntoNewService, IntoService, NewService, Service};
use futures::{Async, Future, Poll};

use crate::connect::{Connect, ConnectResult};
use crate::dispatcher::FramedDispatcher;
use crate::error::ServiceError;
use crate::item::Item;
use crate::state::State;

type RequestItem<S, U> = Item<S, U>;
type ResponseItem<U> = Option<<U as Encoder>::Item>;

/// Service builder - structure that follows the builder pattern
/// for building instances for framed services.
pub struct Builder<St, Codec>(PhantomData<(St, Codec)>);

impl<St, Codec> Builder<St, Codec> {
    pub fn new() -> Builder<St, Codec> {
        Builder(PhantomData)
    }

    /// Construct framed handler service with specified connect service
    pub fn service<Io, C, F>(self, connect: F) -> ServiceBuilder<St, C, Io, Codec>
    where
        F: IntoService<C>,
        Io: AsyncRead + AsyncWrite,
        C: Service<Request = Connect<Io>, Response = ConnectResult<Io, St, Codec>>,
        Codec: Decoder + Encoder,
    {
        ServiceBuilder {
            connect: connect.into_service(),
            disconnect: None,
            _t: PhantomData,
        }
    }

    /// Construct framed handler new service with specified connect service
    pub fn factory<Io, C, F>(self, connect: F) -> NewServiceBuilder<St, C, Io, Codec>
    where
        F: IntoNewService<C>,
        Io: AsyncRead + AsyncWrite,
        C: NewService<
            Config = (),
            Request = Connect<Io>,
            Response = ConnectResult<Io, St, Codec>,
        >,
        C::Error: 'static,
        C::Future: 'static,
        Codec: Decoder + Encoder,
    {
        NewServiceBuilder {
            connect: connect.into_new_service(),
            disconnect: None,
            _t: PhantomData,
        }
    }
}

pub struct ServiceBuilder<St, C, Io, Codec> {
    connect: C,
    disconnect: Option<Rc<dyn Fn(&mut St, bool)>>,
    _t: PhantomData<(St, Io, Codec)>,
}

impl<St, C, Io, Codec> ServiceBuilder<St, C, Io, Codec>
where
    St: 'static,
    Io: AsyncRead + AsyncWrite,
    C: Service<Request = Connect<Io>, Response = ConnectResult<Io, St, Codec>>,
    C::Error: 'static,
    Codec: Decoder + Encoder,
    <Codec as Encoder>::Item: 'static,
    <Codec as Encoder>::Error: std::fmt::Debug,
{
    /// Callback to execute on disconnect
    ///
    /// Second parameter indicates error occured during disconnect.
    pub fn disconnect<F, Out>(mut self, disconnect: F) -> Self
    where
        F: Fn(&mut St, bool) + 'static,
    {
        self.disconnect = Some(Rc::new(disconnect));
        self
    }

    /// Provide stream items handler service and construct service factory.
    pub fn finish<F, T>(
        self,
        service: F,
    ) -> impl Service<Request = Io, Response = (), Error = ServiceError<C::Error, Codec>>
    where
        F: IntoNewService<T>,
        T: NewService<
                Config = St,
                Request = RequestItem<St, Codec>,
                Response = ResponseItem<Codec>,
                Error = C::Error,
                InitError = C::Error,
            > + 'static,
    {
        FramedServiceImpl {
            connect: self.connect,
            handler: Rc::new(service.into_new_service()),
            disconnect: self.disconnect.clone(),
            _t: PhantomData,
        }
    }
}

pub struct NewServiceBuilder<St, C, Io, Codec> {
    connect: C,
    disconnect: Option<Rc<dyn Fn(&mut St, bool)>>,
    _t: PhantomData<(St, Io, Codec)>,
}

impl<St, C, Io, Codec> NewServiceBuilder<St, C, Io, Codec>
where
    St: 'static,
    Io: AsyncRead + AsyncWrite,
    C: NewService<Config = (), Request = Connect<Io>, Response = ConnectResult<Io, St, Codec>>,
    C::Error: 'static,
    C::Future: 'static,
    Codec: Decoder + Encoder,
    <Codec as Encoder>::Item: 'static,
    <Codec as Encoder>::Error: std::fmt::Debug,
{
    /// Callback to execute on disconnect
    ///
    /// Second parameter indicates error occured during disconnect.
    pub fn disconnect<F>(mut self, disconnect: F) -> Self
    where
        F: Fn(&mut St, bool) + 'static,
    {
        self.disconnect = Some(Rc::new(disconnect));
        self
    }

    pub fn finish<F, T, Cfg>(
        self,
        service: F,
    ) -> impl NewService<
        Config = Cfg,
        Request = Io,
        Response = (),
        Error = ServiceError<C::Error, Codec>,
    >
    where
        F: IntoNewService<T>,
        T: NewService<
                Config = St,
                Request = RequestItem<St, Codec>,
                Response = ResponseItem<Codec>,
                Error = C::Error,
                InitError = C::Error,
            > + 'static,
    {
        FramedService {
            connect: self.connect,
            handler: Rc::new(service.into_new_service()),
            disconnect: self.disconnect,
            _t: PhantomData,
        }
    }
}

pub(crate) struct FramedService<St, C, T, Io, Codec, Cfg> {
    connect: C,
    handler: Rc<T>,
    disconnect: Option<Rc<dyn Fn(&mut St, bool)>>,
    _t: PhantomData<(St, Io, Codec, Cfg)>,
}

impl<St, C, T, Io, Codec, Cfg> NewService for FramedService<St, C, T, Io, Codec, Cfg>
where
    St: 'static,
    Io: AsyncRead + AsyncWrite,
    C: NewService<Config = (), Request = Connect<Io>, Response = ConnectResult<Io, St, Codec>>,
    C::Error: 'static,
    C::Future: 'static,
    T: NewService<
            Config = St,
            Request = RequestItem<St, Codec>,
            Response = ResponseItem<Codec>,
            Error = C::Error,
            InitError = C::Error,
        > + 'static,
    Codec: Decoder + Encoder,
    <Codec as Encoder>::Item: 'static,
    <Codec as Encoder>::Error: std::fmt::Debug,
{
    type Config = Cfg;
    type Request = Io;
    type Response = ();
    type Error = ServiceError<C::Error, Codec>;
    type InitError = C::InitError;
    type Service = FramedServiceImpl<St, C::Service, T, Io, Codec>;
    type Future = Box<dyn Future<Item = Self::Service, Error = Self::InitError>>;

    fn new_service(&self, _: &Cfg) -> Self::Future {
        let handler = self.handler.clone();
        let disconnect = self.disconnect.clone();

        // create connect service and then create service impl
        Box::new(
            self.connect
                .new_service(&())
                .map(move |connect| FramedServiceImpl {
                    connect,
                    handler,
                    disconnect,
                    _t: PhantomData,
                }),
        )
    }
}

pub struct FramedServiceImpl<St, C, T, Io, Codec> {
    connect: C,
    handler: Rc<T>,
    disconnect: Option<Rc<dyn Fn(&mut St, bool)>>,
    _t: PhantomData<(St, Io, Codec)>,
}

impl<St, C, T, Io, Codec> Service for FramedServiceImpl<St, C, T, Io, Codec>
where
    Io: AsyncRead + AsyncWrite,
    C: Service<Request = Connect<Io>, Response = ConnectResult<Io, St, Codec>>,
    C::Error: 'static,
    T: NewService<
        Config = St,
        Request = RequestItem<St, Codec>,
        Response = ResponseItem<Codec>,
        Error = C::Error,
        InitError = C::Error,
    >,
    <<T as NewService>::Service as Service>::Future: 'static,
    Codec: Decoder + Encoder,
    <Codec as Encoder>::Item: 'static,
    <Codec as Encoder>::Error: std::fmt::Debug,
{
    type Request = Io;
    type Response = ();
    type Error = ServiceError<C::Error, Codec>;
    type Future = FramedServiceImplResponse<St, Io, Codec, C, T>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.connect.poll_ready().map_err(|e| e.into())
    }

    fn call(&mut self, req: Io) -> Self::Future {
        FramedServiceImplResponse {
            inner: FramedServiceImplResponseInner::Connect(
                self.connect.call(Connect::new(req)),
                self.handler.clone(),
            ),
            disconnect: self.disconnect.clone(),
        }
    }
}

pub struct FramedServiceImplResponse<St, Io, Codec, C, T>
where
    C: Service<Request = Connect<Io>, Response = ConnectResult<Io, St, Codec>>,
    C::Error: 'static,
    T: NewService<
        Config = St,
        Request = RequestItem<St, Codec>,
        Response = ResponseItem<Codec>,
        Error = C::Error,
        InitError = C::Error,
    >,
    <<T as NewService>::Service as Service>::Future: 'static,
    Io: AsyncRead + AsyncWrite,
    Codec: Encoder + Decoder,
    <Codec as Encoder>::Item: 'static,
    <Codec as Encoder>::Error: std::fmt::Debug,
{
    inner: FramedServiceImplResponseInner<St, Io, Codec, C, T>,
    disconnect: Option<Rc<dyn Fn(&mut St, bool)>>,
}

enum FramedServiceImplResponseInner<St, Io, Codec, C, T>
where
    C: Service<Request = Connect<Io>, Response = ConnectResult<Io, St, Codec>>,
    C::Error: 'static,
    T: NewService<
        Config = St,
        Request = RequestItem<St, Codec>,
        Response = ResponseItem<Codec>,
        Error = C::Error,
        InitError = C::Error,
    >,
    <<T as NewService>::Service as Service>::Future: 'static,
    Io: AsyncRead + AsyncWrite,
    Codec: Encoder + Decoder,
    <Codec as Encoder>::Item: 'static,
    <Codec as Encoder>::Error: std::fmt::Debug,
{
    Connect(C::Future, Rc<T>),
    Handler(T::Future, Option<ConnectResult<Io, St, Codec>>),
    Dispatcher(FramedDispatcher<St, T::Service, Io, Codec>),
}

impl<St, Io, Codec, C, T> Future for FramedServiceImplResponse<St, Io, Codec, C, T>
where
    C: Service<Request = Connect<Io>, Response = ConnectResult<Io, St, Codec>>,
    C::Error: 'static,
    T: NewService<
        Config = St,
        Request = RequestItem<St, Codec>,
        Response = ResponseItem<Codec>,
        Error = C::Error,
        InitError = C::Error,
    >,
    <<T as NewService>::Service as Service>::Future: 'static,
    Io: AsyncRead + AsyncWrite,
    Codec: Encoder + Decoder,
    <Codec as Encoder>::Item: 'static,
    <Codec as Encoder>::Error: std::fmt::Debug,
{
    type Item = ();
    type Error = ServiceError<C::Error, Codec>;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.inner {
            FramedServiceImplResponseInner::Connect(ref mut fut, ref handler) => {
                match fut.poll()? {
                    Async::Ready(res) => {
                        self.inner = FramedServiceImplResponseInner::Handler(
                            handler.new_service(&res.state),
                            Some(res),
                        );
                        self.poll()
                    }
                    Async::NotReady => Ok(Async::NotReady),
                }
            }
            FramedServiceImplResponseInner::Handler(ref mut fut, ref mut res) => {
                match fut.poll()? {
                    Async::Ready(handler) => {
                        let res = res.take().unwrap();
                        self.inner =
                            FramedServiceImplResponseInner::Dispatcher(FramedDispatcher::new(
                                res.framed,
                                State::new(res.state),
                                handler,
                                res.rx,
                                res.sink,
                                self.disconnect.clone(),
                            ));
                        self.poll()
                    }
                    Async::NotReady => Ok(Async::NotReady),
                }
            }
            FramedServiceImplResponseInner::Dispatcher(ref mut fut) => fut.poll(),
        }
    }
}
