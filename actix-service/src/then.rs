use alloc::rc::Rc;
use core::{
    future::Future,
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

use futures_core::ready;
use pin_project_lite::pin_project;

use super::{Service, ServiceFactory};

/// Service for the `then` combinator, chaining a computation onto the end of
/// another service.
///
/// This is created by the `Pipeline::then` method.
pub(crate) struct ThenService<A, B, Req>(Rc<(A, B)>, PhantomData<Req>);

impl<A, B, Req> ThenService<A, B, Req> {
    /// Create new `.then()` combinator
    pub(crate) fn new(a: A, b: B) -> ThenService<A, B, Req>
    where
        A: Service<Req>,
        B: Service<Result<A::Response, A::Error>, Error = A::Error>,
    {
        Self(Rc::new((a, b)), PhantomData)
    }
}

impl<A, B, Req> Clone for ThenService<A, B, Req> {
    fn clone(&self) -> Self {
        ThenService(self.0.clone(), PhantomData)
    }
}

impl<A, B, Req> Service<Req> for ThenService<A, B, Req>
where
    A: Service<Req>,
    B: Service<Result<A::Response, A::Error>, Error = A::Error>,
{
    type Response = B::Response;
    type Error = B::Error;
    type Future = ThenServiceResponse<A, B, Req>;

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let (a, b) = &*self.0;
        let not_ready = !a.poll_ready(cx)?.is_ready();
        if !b.poll_ready(cx)?.is_ready() || not_ready {
            Poll::Pending
        } else {
            Poll::Ready(Ok(()))
        }
    }

    fn call(&self, req: Req) -> Self::Future {
        ThenServiceResponse {
            state: State::A {
                fut: self.0 .0.call(req),
                b: Some(self.0.clone()),
            },
        }
    }
}

pin_project! {
    pub(crate) struct ThenServiceResponse<A, B, Req>
    where
        A: Service<Req>,
        B: Service<Result<A::Response, A::Error>>,
    {
        #[pin]
        state: State<A, B, Req>,
    }
}

pin_project! {
    #[project = StateProj]
    enum State<A, B, Req>
    where
        A: Service<Req>,
        B: Service<Result<A::Response, A::Error>>,
    {
        A { #[pin] fut: A::Future, b: Option<Rc<(A, B)>> },
        B { #[pin] fut: B::Future },
    }
}

impl<A, B, Req> Future for ThenServiceResponse<A, B, Req>
where
    A: Service<Req>,
    B: Service<Result<A::Response, A::Error>>,
{
    type Output = Result<B::Response, B::Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.as_mut().project();

        match this.state.as_mut().project() {
            StateProj::A { fut, b } => {
                let res = ready!(fut.poll(cx));
                let b = b.take().unwrap();
                let fut = b.1.call(res);
                this.state.set(State::B { fut });
                self.poll(cx)
            }
            StateProj::B { fut } => fut.poll(cx),
        }
    }
}

/// `.then()` service factory combinator
pub(crate) struct ThenServiceFactory<A, B, Req>(Rc<(A, B)>, PhantomData<Req>);

impl<A, B, Req> ThenServiceFactory<A, B, Req>
where
    A: ServiceFactory<Req>,
    A::Config: Clone,
    B: ServiceFactory<
        Result<A::Response, A::Error>,
        Config = A::Config,
        Error = A::Error,
        InitError = A::InitError,
    >,
{
    /// Create new `AndThen` combinator
    pub(crate) fn new(a: A, b: B) -> Self {
        Self(Rc::new((a, b)), PhantomData)
    }
}

impl<A, B, Req> ServiceFactory<Req> for ThenServiceFactory<A, B, Req>
where
    A: ServiceFactory<Req>,
    A::Config: Clone,
    B: ServiceFactory<
        Result<A::Response, A::Error>,
        Config = A::Config,
        Error = A::Error,
        InitError = A::InitError,
    >,
{
    type Response = B::Response;
    type Error = A::Error;

    type Config = A::Config;
    type Service = ThenService<A::Service, B::Service, Req>;
    type InitError = A::InitError;
    type Future = ThenServiceFactoryResponse<A, B, Req>;

    fn new_service(&self, cfg: A::Config) -> Self::Future {
        let srv = &*self.0;
        ThenServiceFactoryResponse::new(srv.0.new_service(cfg.clone()), srv.1.new_service(cfg))
    }
}

impl<A, B, Req> Clone for ThenServiceFactory<A, B, Req> {
    fn clone(&self) -> Self {
        Self(self.0.clone(), PhantomData)
    }
}

pin_project! {
    pub(crate) struct ThenServiceFactoryResponse<A, B, Req>
    where
        A: ServiceFactory<Req>,
        B: ServiceFactory<
            Result<A::Response, A::Error>,
            Config = A::Config,
            Error = A::Error,
            InitError = A::InitError,
        >,
    {
        #[pin]
        fut_b: B::Future,
        #[pin]
        fut_a: A::Future,
        a: Option<A::Service>,
        b: Option<B::Service>,
    }
}

impl<A, B, Req> ThenServiceFactoryResponse<A, B, Req>
where
    A: ServiceFactory<Req>,
    B: ServiceFactory<
        Result<A::Response, A::Error>,
        Config = A::Config,
        Error = A::Error,
        InitError = A::InitError,
    >,
{
    fn new(fut_a: A::Future, fut_b: B::Future) -> Self {
        Self {
            fut_a,
            fut_b,
            a: None,
            b: None,
        }
    }
}

impl<A, B, Req> Future for ThenServiceFactoryResponse<A, B, Req>
where
    A: ServiceFactory<Req>,
    B: ServiceFactory<
        Result<A::Response, A::Error>,
        Config = A::Config,
        Error = A::Error,
        InitError = A::InitError,
    >,
{
    type Output = Result<ThenService<A::Service, B::Service, Req>, A::InitError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        if this.a.is_none() {
            if let Poll::Ready(service) = this.fut_a.poll(cx)? {
                *this.a = Some(service);
            }
        }
        if this.b.is_none() {
            if let Poll::Ready(service) = this.fut_b.poll(cx)? {
                *this.b = Some(service);
            }
        }
        if this.a.is_some() && this.b.is_some() {
            Poll::Ready(Ok(ThenService::new(
                this.a.take().unwrap(),
                this.b.take().unwrap(),
            )))
        } else {
            Poll::Pending
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::rc::Rc;
    use core::{
        cell::Cell,
        task::{Context, Poll},
    };

    use futures_util::future::lazy;

    use crate::{
        err, ok,
        pipeline::{pipeline, pipeline_factory},
        ready, Ready, Service, ServiceFactory,
    };

    #[derive(Clone)]
    struct Srv1(Rc<Cell<usize>>);

    impl Service<Result<&'static str, &'static str>> for Srv1 {
        type Response = &'static str;
        type Error = ();
        type Future = Ready<Result<Self::Response, Self::Error>>;

        fn poll_ready(&self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            self.0.set(self.0.get() + 1);
            Poll::Ready(Ok(()))
        }

        fn call(&self, req: Result<&'static str, &'static str>) -> Self::Future {
            match req {
                Ok(msg) => ok(msg),
                Err(_) => err(()),
            }
        }
    }

    struct Srv2(Rc<Cell<usize>>);

    impl Service<Result<&'static str, ()>> for Srv2 {
        type Response = (&'static str, &'static str);
        type Error = ();
        type Future = Ready<Result<Self::Response, ()>>;

        fn poll_ready(&self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            self.0.set(self.0.get() + 1);
            Poll::Ready(Err(()))
        }

        fn call(&self, req: Result<&'static str, ()>) -> Self::Future {
            match req {
                Ok(msg) => ok((msg, "ok")),
                Err(()) => ok(("srv2", "err")),
            }
        }
    }

    #[actix_rt::test]
    async fn test_poll_ready() {
        let cnt = Rc::new(Cell::new(0));
        let srv = pipeline(Srv1(cnt.clone())).then(Srv2(cnt.clone()));
        let res = lazy(|cx| srv.poll_ready(cx)).await;
        assert_eq!(res, Poll::Ready(Err(())));
        assert_eq!(cnt.get(), 2);
    }

    #[actix_rt::test]
    async fn test_call() {
        let cnt = Rc::new(Cell::new(0));
        let srv = pipeline(Srv1(cnt.clone())).then(Srv2(cnt));

        let res = srv.call(Ok("srv1")).await;
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), ("srv1", "ok"));

        let res = srv.call(Err("srv")).await;
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), ("srv2", "err"));
    }

    #[actix_rt::test]
    async fn test_factory() {
        let cnt = Rc::new(Cell::new(0));
        let cnt2 = cnt.clone();
        let blank = move || ready(Ok::<_, ()>(Srv1(cnt2.clone())));
        let factory = pipeline_factory(blank).then(move || ready(Ok(Srv2(cnt.clone()))));
        let srv = factory.new_service(&()).await.unwrap();
        let res = srv.call(Ok("srv1")).await;
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), ("srv1", "ok"));

        let res = srv.call(Err("srv")).await;
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), ("srv2", "err"));
    }
}
