use std::marker::PhantomData;

use futures::{Future, Poll};

use super::{NewService, Service};

use pin_project::pin_project;
use std::pin::Pin;
use std::task::Context;

/// Service for the `map_err` combinator, changing the type of a service's
/// error.
///
/// This is created by the `ServiceExt::map_err` method.
#[pin_project]
pub struct MapErr<A, F, E> {
    #[pin]
    service: A,
    f: F,
    _t: PhantomData<E>,
}

impl<A, F, E> MapErr<A, F, E> {
    /// Create new `MapErr` combinator
    pub fn new(service: A, f: F) -> Self
    where
        A: Service,
        F: Fn(A::Error) -> E,
    {
        Self {
            service,
            f,
            _t: PhantomData,
        }
    }
}

impl<A, F, E> Clone for MapErr<A, F, E>
where
    A: Clone,
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

impl<A, F, E> Service for MapErr<A, F, E>
where
    A: Service,
    F: Fn(A::Error) -> E + Clone,
{
    type Request = A::Request;
    type Response = A::Response;
    type Error = E;
    type Future = MapErrFuture<A, F, E>;

    fn poll_ready(
        mut self: Pin<&mut Self>,
        ctx: &mut Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        let mut this = self.project();
        this.service.poll_ready(ctx).map_err(this.f)
    }

    fn call(&mut self, req: A::Request) -> Self::Future {
        MapErrFuture::new(self.service.call(req), self.f.clone())
    }
}

#[pin_project]
pub struct MapErrFuture<A, F, E>
where
    A: Service,
    F: Fn(A::Error) -> E,
{
    f: F,
    #[pin]
    fut: A::Future,
}

impl<A, F, E> MapErrFuture<A, F, E>
where
    A: Service,
    F: Fn(A::Error) -> E,
{
    fn new(fut: A::Future, f: F) -> Self {
        MapErrFuture { f, fut }
    }
}

impl<A, F, E> Future for MapErrFuture<A, F, E>
where
    A: Service,
    F: Fn(A::Error) -> E,
{
    type Output = Result<A::Response, E>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();
        this.fut.poll(cx).map_err(this.f)
    }
}

/// NewService for the `map_err` combinator, changing the type of a new
/// service's error.
///
/// This is created by the `NewServiceExt::map_err` method.
pub struct MapErrNewService<A, F, E>
where
    A: NewService,
    F: Fn(A::Error) -> E + Clone,
{
    a: A,
    f: F,
    e: PhantomData<E>,
}

impl<A, F, E> MapErrNewService<A, F, E>
where
    A: NewService,
    F: Fn(A::Error) -> E + Clone,
{
    /// Create new `MapErr` new service instance
    pub fn new(a: A, f: F) -> Self {
        Self {
            a,
            f,
            e: PhantomData,
        }
    }
}

impl<A, F, E> Clone for MapErrNewService<A, F, E>
where
    A: NewService + Clone,
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

impl<A, F, E> NewService for MapErrNewService<A, F, E>
where
    A: NewService,
    F: Fn(A::Error) -> E + Clone,
{
    type Request = A::Request;
    type Response = A::Response;
    type Error = E;

    type Config = A::Config;
    type Service = MapErr<A::Service, F, E>;
    type InitError = A::InitError;
    type Future = MapErrNewServiceFuture<A, F, E>;

    fn new_service(&self, cfg: &A::Config) -> Self::Future {
        MapErrNewServiceFuture::new(self.a.new_service(cfg), self.f.clone())
    }
}

#[pin_project]
pub struct MapErrNewServiceFuture<A, F, E>
where
    A: NewService,
    F: Fn(A::Error) -> E,
{
    #[pin]
    fut: A::Future,
    f: F,
}

impl<A, F, E> MapErrNewServiceFuture<A, F, E>
where
    A: NewService,
    F: Fn(A::Error) -> E,
{
    fn new(fut: A::Future, f: F) -> Self {
        MapErrNewServiceFuture { f, fut }
    }
}

impl<A, F, E> Future for MapErrNewServiceFuture<A, F, E>
where
    A: NewService,
    F: Fn(A::Error) -> E + Clone,
{
    type Output = Result<MapErr<A::Service, F, E>, A::InitError>;

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
    use futures::future::{err, Ready};

    use super::*;
    use crate::{IntoNewService, NewService, Service, ServiceExt};
    use tokio::future::ok;

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
            Poll::Ready(Err(()))
        }

        fn call(&mut self, _: ()) -> Self::Future {
            err(())
        }
    }

    #[tokio::test]
    async fn test_poll_ready() {
        let mut srv = Srv.map_err(|_| "error");
        let res = srv.poll_once().await;
        if let Poll::Ready(res) = res {
            assert!(res.is_err());
            assert_eq!(res.err().unwrap(), "error");
        } else {
            panic!("Should be ready");
        }
    }

    #[tokio::test]
    async fn test_call() {
        let mut srv = Srv.map_err(|_| "error");
        let res = srv.call(()).await;
        assert!(res.is_err());
        assert_eq!(res.err().unwrap(), "error");
    }

    #[tokio::test]
    async fn test_new_service() {
        let blank = || ok::<_, ()>(Srv);
        let new_srv = blank.into_new_service().map_err(|_| "error");
        let mut srv = new_srv.new_service(&()).await.unwrap();
        let res = srv.call(()).await;
        assert!(res.is_err());
        assert_eq!(res.err().unwrap(), "error");
    }
}
