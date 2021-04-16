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

/// Service for the `and_then` combinator, chaining a computation onto the end of another service
/// which completes successfully.
///
/// This is created by the `Pipeline::and_then` method.
pub struct AndThenService<A, B, Req>(Rc<(A, B)>, PhantomData<Req>);

impl<A, B, Req> AndThenService<A, B, Req> {
    /// Create new `AndThen` combinator
    pub(crate) fn new(a: A, b: B) -> Self
    where
        A: Service<Req>,
        B: Service<A::Response, Error = A::Error>,
    {
        Self(Rc::new((a, b)), PhantomData)
    }
}

impl<A, B, Req> Clone for AndThenService<A, B, Req> {
    fn clone(&self) -> Self {
        AndThenService(self.0.clone(), PhantomData)
    }
}

impl<A, B, Req> Service<Req> for AndThenService<A, B, Req>
where
    A: Service<Req>,
    B: Service<A::Response, Error = A::Error>,
{
    type Response = B::Response;
    type Error = A::Error;
    type Future = AndThenServiceResponse<A, B, Req>;

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
        AndThenServiceResponse {
            state: State::A {
                fut: self.0 .0.call(req),
                b: Some(self.0.clone()),
            },
        }
    }
}

pin_project! {
    pub struct AndThenServiceResponse<A, B, Req>
    where
        A: Service<Req>,
        B: Service<A::Response, Error = A::Error>,
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
        B: Service<A::Response, Error = A::Error>,
    {
        A {
            #[pin]
            fut: A::Future,
            b: Option<Rc<(A, B)>>,
        },
        B {
            #[pin]
            fut: B::Future,
        },
    }
}

impl<A, B, Req> Future for AndThenServiceResponse<A, B, Req>
where
    A: Service<Req>,
    B: Service<A::Response, Error = A::Error>,
{
    type Output = Result<B::Response, A::Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.as_mut().project();

        match this.state.as_mut().project() {
            StateProj::A { fut, b } => {
                let res = ready!(fut.poll(cx))?;
                let b = b.take().unwrap();
                let fut = b.1.call(res);
                this.state.set(State::B { fut });
                self.poll(cx)
            }
            StateProj::B { fut } => fut.poll(cx),
        }
    }
}

/// `.and_then()` service factory combinator
pub struct AndThenServiceFactory<A, B, Req>
where
    A: ServiceFactory<Req>,
    A::Config: Clone,
    B: ServiceFactory<
        A::Response,
        Config = A::Config,
        Error = A::Error,
        InitError = A::InitError,
    >,
{
    inner: Rc<(A, B)>,
    _phantom: PhantomData<Req>,
}

impl<A, B, Req> AndThenServiceFactory<A, B, Req>
where
    A: ServiceFactory<Req>,
    A::Config: Clone,
    B: ServiceFactory<
        A::Response,
        Config = A::Config,
        Error = A::Error,
        InitError = A::InitError,
    >,
{
    /// Create new `AndThenFactory` combinator
    pub(crate) fn new(a: A, b: B) -> Self {
        Self {
            inner: Rc::new((a, b)),
            _phantom: PhantomData,
        }
    }
}

impl<A, B, Req> ServiceFactory<Req> for AndThenServiceFactory<A, B, Req>
where
    A: ServiceFactory<Req>,
    A::Config: Clone,
    B: ServiceFactory<
        A::Response,
        Config = A::Config,
        Error = A::Error,
        InitError = A::InitError,
    >,
{
    type Response = B::Response;
    type Error = A::Error;

    type Config = A::Config;
    type Service = AndThenService<A::Service, B::Service, Req>;
    type InitError = A::InitError;
    type Future = AndThenServiceFactoryResponse<A, B, Req>;

    fn new_service(&self, cfg: A::Config) -> Self::Future {
        let inner = &*self.inner;
        AndThenServiceFactoryResponse::new(
            inner.0.new_service(cfg.clone()),
            inner.1.new_service(cfg),
        )
    }
}

impl<A, B, Req> Clone for AndThenServiceFactory<A, B, Req>
where
    A: ServiceFactory<Req>,
    A::Config: Clone,
    B: ServiceFactory<
        A::Response,
        Config = A::Config,
        Error = A::Error,
        InitError = A::InitError,
    >,
{
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            _phantom: PhantomData,
        }
    }
}

pin_project! {
    pub struct AndThenServiceFactoryResponse<A, B, Req>
    where
        A: ServiceFactory<Req>,
        B: ServiceFactory<A::Response>,
    {
        #[pin]
        fut_a: A::Future,
        #[pin]
        fut_b: B::Future,

        a: Option<A::Service>,
        b: Option<B::Service>,
    }
}

impl<A, B, Req> AndThenServiceFactoryResponse<A, B, Req>
where
    A: ServiceFactory<Req>,
    B: ServiceFactory<A::Response>,
{
    fn new(fut_a: A::Future, fut_b: B::Future) -> Self {
        AndThenServiceFactoryResponse {
            fut_a,
            fut_b,
            a: None,
            b: None,
        }
    }
}

impl<A, B, Req> Future for AndThenServiceFactoryResponse<A, B, Req>
where
    A: ServiceFactory<Req>,
    B: ServiceFactory<A::Response, Error = A::Error, InitError = A::InitError>,
{
    type Output = Result<AndThenService<A::Service, B::Service, Req>, A::InitError>;

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
            Poll::Ready(Ok(AndThenService::new(
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
        fn_factory, ok,
        pipeline::{pipeline, pipeline_factory},
        ready, Ready, Service, ServiceFactory,
    };

    struct Srv1(Rc<Cell<usize>>);

    impl Service<&'static str> for Srv1 {
        type Response = &'static str;
        type Error = ();
        type Future = Ready<Result<Self::Response, ()>>;

        fn poll_ready(&self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            self.0.set(self.0.get() + 1);
            Poll::Ready(Ok(()))
        }

        fn call(&self, req: &'static str) -> Self::Future {
            ok(req)
        }
    }

    #[derive(Clone)]
    struct Srv2(Rc<Cell<usize>>);

    impl Service<&'static str> for Srv2 {
        type Response = (&'static str, &'static str);
        type Error = ();
        type Future = Ready<Result<Self::Response, ()>>;

        fn poll_ready(&self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            self.0.set(self.0.get() + 1);
            Poll::Ready(Ok(()))
        }

        fn call(&self, req: &'static str) -> Self::Future {
            ok((req, "srv2"))
        }
    }

    #[actix_rt::test]
    async fn test_poll_ready() {
        let cnt = Rc::new(Cell::new(0));
        let srv = pipeline(Srv1(cnt.clone())).and_then(Srv2(cnt.clone()));
        let res = lazy(|cx| srv.poll_ready(cx)).await;
        assert_eq!(res, Poll::Ready(Ok(())));
        assert_eq!(cnt.get(), 2);
    }

    #[actix_rt::test]
    async fn test_call() {
        let cnt = Rc::new(Cell::new(0));
        let srv = pipeline(Srv1(cnt.clone())).and_then(Srv2(cnt));
        let res = srv.call("srv1").await;
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), ("srv1", "srv2"));
    }

    #[actix_rt::test]
    async fn test_new_service() {
        let cnt = Rc::new(Cell::new(0));
        let cnt2 = cnt.clone();
        let new_srv =
            pipeline_factory(fn_factory(move || ready(Ok::<_, ()>(Srv1(cnt2.clone())))))
                .and_then(move || ready(Ok(Srv2(cnt.clone()))));

        let srv = new_srv.new_service(()).await.unwrap();
        let res = srv.call("srv1").await;
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), ("srv1", "srv2"));
    }
}
