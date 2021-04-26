use core::{
    future::Future,
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

use pin_project_lite::pin_project;

use super::{Service, ServiceFactory};

/// Service for the `map_err` combinator, changing the type of a service's
/// error.
///
/// This is created by the `ServiceExt::map_err` method.
pub struct MapErr<S, Req, F, E> {
    service: S,
    f: F,
    _t: PhantomData<(E, Req)>,
}

impl<S, Req, F, E> MapErr<S, Req, F, E> {
    /// Create new `MapErr` combinator
    pub(crate) fn new(service: S, f: F) -> Self
    where
        S: Service<Req>,
        F: Fn(S::Error) -> E,
    {
        Self {
            service,
            f,
            _t: PhantomData,
        }
    }
}

impl<S, Req, F, E> Clone for MapErr<S, Req, F, E>
where
    S: Clone,
    F: Clone,
{
    fn clone(&self) -> Self {
        MapErr {
            service: self.service.clone(),
            f: self.f.clone(),
            _t: PhantomData,
        }
    }
}

impl<A, Req, F, E> Service<Req> for MapErr<A, Req, F, E>
where
    A: Service<Req>,
    F: Fn(A::Error) -> E + Clone,
{
    type Response = A::Response;
    type Error = E;
    type Future = MapErrFuture<A, Req, F, E>;

    fn poll_ready(&self, ctx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(ctx).map_err(&self.f)
    }

    fn call(&self, req: Req) -> Self::Future {
        MapErrFuture::new(self.service.call(req), self.f.clone())
    }
}

pin_project! {
    pub struct MapErrFuture<A, Req, F, E>
    where
        A: Service<Req>,
        F: Fn(A::Error) -> E,
    {
        f: F,
        #[pin]
        fut: A::Future,
    }
}

impl<A, Req, F, E> MapErrFuture<A, Req, F, E>
where
    A: Service<Req>,
    F: Fn(A::Error) -> E,
{
    fn new(fut: A::Future, f: F) -> Self {
        MapErrFuture { f, fut }
    }
}

impl<A, Req, F, E> Future for MapErrFuture<A, Req, F, E>
where
    A: Service<Req>,
    F: Fn(A::Error) -> E,
{
    type Output = Result<A::Response, E>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        this.fut.poll(cx).map_err(this.f)
    }
}

/// Factory for the `map_err` combinator, changing the type of a new
/// service's error.
///
/// This is created by the `NewServiceExt::map_err` method.
pub struct MapErrServiceFactory<A, Req, F, E>
where
    A: ServiceFactory<Req>,
    F: Fn(A::Error) -> E + Clone,
{
    a: A,
    f: F,
    e: PhantomData<(E, Req)>,
}

impl<A, Req, F, E> MapErrServiceFactory<A, Req, F, E>
where
    A: ServiceFactory<Req>,
    F: Fn(A::Error) -> E + Clone,
{
    /// Create new `MapErr` new service instance
    pub(crate) fn new(a: A, f: F) -> Self {
        Self {
            a,
            f,
            e: PhantomData,
        }
    }
}

impl<A, Req, F, E> Clone for MapErrServiceFactory<A, Req, F, E>
where
    A: ServiceFactory<Req> + Clone,
    F: Fn(A::Error) -> E + Clone,
{
    fn clone(&self) -> Self {
        Self {
            a: self.a.clone(),
            f: self.f.clone(),
            e: PhantomData,
        }
    }
}

impl<A, Req, F, E> ServiceFactory<Req> for MapErrServiceFactory<A, Req, F, E>
where
    A: ServiceFactory<Req>,
    F: Fn(A::Error) -> E + Clone,
{
    type Response = A::Response;
    type Error = E;

    type Config = A::Config;
    type Service = MapErr<A::Service, Req, F, E>;
    type InitError = A::InitError;
    type Future = MapErrServiceFuture<A, Req, F, E>;

    fn new_service(&self, cfg: A::Config) -> Self::Future {
        MapErrServiceFuture::new(self.a.new_service(cfg), self.f.clone())
    }
}

pin_project! {
    pub struct MapErrServiceFuture<A, Req, F, E>
    where
        A: ServiceFactory<Req>,
        F: Fn(A::Error) -> E,
    {
        #[pin]
        fut: A::Future,
        f: F,
    }
}

impl<A, Req, F, E> MapErrServiceFuture<A, Req, F, E>
where
    A: ServiceFactory<Req>,
    F: Fn(A::Error) -> E,
{
    fn new(fut: A::Future, f: F) -> Self {
        MapErrServiceFuture { fut, f }
    }
}

impl<A, Req, F, E> Future for MapErrServiceFuture<A, Req, F, E>
where
    A: ServiceFactory<Req>,
    F: Fn(A::Error) -> E + Clone,
{
    type Output = Result<MapErr<A::Service, Req, F, E>, A::InitError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        if let Poll::Ready(svc) = this.fut.poll(cx)? {
            Poll::Ready(Ok(MapErr::new(svc, this.f.clone())))
        } else {
            Poll::Pending
        }
    }
}

#[cfg(test)]
mod tests {
    use futures_util::future::lazy;

    use super::*;
    use crate::{
        err, ok, IntoServiceFactory, Ready, Service, ServiceExt, ServiceFactory,
        ServiceFactoryExt,
    };

    struct Srv;

    impl Service<()> for Srv {
        type Response = ();
        type Error = ();
        type Future = Ready<Result<(), ()>>;

        fn poll_ready(&self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Err(()))
        }

        fn call(&self, _: ()) -> Self::Future {
            err(())
        }
    }

    #[actix_rt::test]
    async fn test_poll_ready() {
        let srv = Srv.map_err(|_| "error");
        let res = lazy(|cx| srv.poll_ready(cx)).await;
        assert_eq!(res, Poll::Ready(Err("error")));
    }

    #[actix_rt::test]
    async fn test_call() {
        let srv = Srv.map_err(|_| "error");
        let res = srv.call(()).await;
        assert!(res.is_err());
        assert_eq!(res.err().unwrap(), "error");
    }

    #[actix_rt::test]
    async fn test_new_service() {
        let new_srv = (|| ok::<_, ()>(Srv)).into_factory().map_err(|_| "error");
        let srv = new_srv.new_service(&()).await.unwrap();
        let res = srv.call(()).await;
        assert!(res.is_err());
        assert_eq!(res.err().unwrap(), "error");
    }
}
