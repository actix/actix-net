use std::marker::PhantomData;

use futures::{Future, Poll};

use super::{NewService, Service};
use std::pin::Pin;
use std::task::Context;

use pin_project::pin_project;

/// Service for the `map` combinator, changing the type of a service's response.
///
/// This is created by the `ServiceExt::map` method.
#[pin_project]
pub struct Map<A, F, Response> {
    #[pin]
    service: A,
    f: F,
    _t: PhantomData<Response>,
}

impl<A, F, Response> Map<A, F, Response> {
    /// Create new `Map` combinator
    pub fn new(service: A, f: F) -> Self
    where
        A: Service,
        F: FnMut(A::Response) -> Response,
    {
        Self {
            service,
            f,
            _t: PhantomData,
        }
    }
}

impl<A, F, Response> Clone for Map<A, F, Response>
where
    A: Clone,
    F: Clone,
{
    fn clone(&self) -> Self {
        Map {
            service: self.service.clone(),
            f: self.f.clone(),
            _t: PhantomData,
        }
    }
}

impl<A, F, Response> Service for Map<A, F, Response>
where
    A: Service,
    F: FnMut(A::Response) -> Response + Clone,
{
    type Request = A::Request;
    type Response = Response;
    type Error = A::Error;
    type Future = MapFuture<A, F, Response>;

    fn poll_ready(
        self: Pin<&mut Self>,
        ctx: &mut Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        self.project().service.poll_ready(ctx)
    }

    fn call(&mut self, req: A::Request) -> Self::Future {
        MapFuture::new(self.service.call(req), self.f.clone())
    }
}

#[pin_project]
pub struct MapFuture<A, F, Response>
where
    A: Service,
    F: FnMut(A::Response) -> Response,
{
    f: F,
    #[pin]
    fut: A::Future,
}

impl<A, F, Response> MapFuture<A, F, Response>
where
    A: Service,
    F: FnMut(A::Response) -> Response,
{
    fn new(fut: A::Future, f: F) -> Self {
        MapFuture { f, fut }
    }
}

impl<A, F, Response> Future for MapFuture<A, F, Response>
where
    A: Service,
    F: FnMut(A::Response) -> Response,
{
    type Output = Result<Response, A::Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        match this.fut.poll(cx) {
            Poll::Ready(Ok(resp)) => Poll::Ready(Ok((this.f)(resp))),
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            Poll::Pending => Poll::Pending,
        }
    }
}

/// `MapNewService` new service combinator
pub struct MapNewService<A, F, Res> {
    a: A,
    f: F,
    r: PhantomData<Res>,
}

impl<A, F, Res> MapNewService<A, F, Res> {
    /// Create new `Map` new service instance
    pub fn new(a: A, f: F) -> Self
    where
        A: NewService,
        F: FnMut(A::Response) -> Res,
    {
        Self {
            a,
            f,
            r: PhantomData,
        }
    }
}

impl<A, F, Res> Clone for MapNewService<A, F, Res>
where
    A: Clone,
    F: Clone,
{
    fn clone(&self) -> Self {
        Self {
            a: self.a.clone(),
            f: self.f.clone(),
            r: PhantomData,
        }
    }
}

impl<A, F, Res> NewService for MapNewService<A, F, Res>
where
    A: NewService,
    F: FnMut(A::Response) -> Res + Clone,
{
    type Request = A::Request;
    type Response = Res;
    type Error = A::Error;

    type Config = A::Config;
    type Service = Map<A::Service, F, Res>;
    type InitError = A::InitError;
    type Future = MapNewServiceFuture<A, F, Res>;

    fn new_service(&self, cfg: &A::Config) -> Self::Future {
        MapNewServiceFuture::new(self.a.new_service(cfg), self.f.clone())
    }
}

#[pin_project]
pub struct MapNewServiceFuture<A, F, Res>
where
    A: NewService,
    F: FnMut(A::Response) -> Res,
{
    #[pin]
    fut: A::Future,
    f: Option<F>,
}

impl<A, F, Res> MapNewServiceFuture<A, F, Res>
where
    A: NewService,
    F: FnMut(A::Response) -> Res,
{
    fn new(fut: A::Future, f: F) -> Self {
        MapNewServiceFuture { f: Some(f), fut }
    }
}

impl<A, F, Res> Future for MapNewServiceFuture<A, F, Res>
where
    A: NewService,
    F: FnMut(A::Response) -> Res,
{
    type Output = Result<Map<A::Service, F, Res>, A::InitError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        if let Poll::Ready(svc) = this.fut.poll(cx)? {
            Poll::Ready(Ok(Map::new(svc, this.f.take().unwrap())))
        } else {
            Poll::Pending
        }
    }
}

#[cfg(test)]
mod tests {
    use futures::future::{ok, Ready};

    use super::*;
    use crate::{IntoNewService, Service, ServiceExt};

    struct Srv;

    impl Service for Srv {
        type Request = ();
        type Response = ();
        type Error = ();
        type Future = Ready<Result<(), ()>>;

        fn poll_ready(
            self: Pin<&mut Self>,
            ctx: &mut Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, _: ()) -> Self::Future {
            ok(())
        }
    }

    #[tokio::test]
    async fn test_poll_ready() {
        let mut srv = Srv.map(|_| "ok");
        let res = srv.poll_once().await;
        assert_eq!(res, Poll::Ready(Ok(())));
    }

    #[tokio::test]
    async fn test_call() {
        let mut srv = Srv.map(|_| "ok");
        let res = srv.call(()).await;
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), "ok");
    }

    #[tokio::test]
    async fn test_new_service() {
        let blank = || ok::<_, ()>(Srv);
        let new_srv = blank.into_new_service().map(|_| "ok");
        let mut srv = new_srv.new_service(&()).await.unwrap();
        let res = srv.call(()).await;
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), ("ok"));
    }
}
