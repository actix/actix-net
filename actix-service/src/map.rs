use core::{
    future::Future,
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

use pin_project_lite::pin_project;

use super::{Service, ServiceFactory};

/// Service for the `map` combinator, changing the type of a service's response.
///
/// This is created by the `ServiceExt::map` method.
pub struct Map<A, F, Req, Res> {
    service: A,
    f: F,
    _t: PhantomData<fn(Req) -> Res>,
}

impl<A, F, Req, Res> Map<A, F, Req, Res> {
    /// Create new `Map` combinator
    pub(crate) fn new(service: A, f: F) -> Self
    where
        A: Service<Req>,
        F: FnMut(A::Response) -> Res,
    {
        Self {
            service,
            f,
            _t: PhantomData,
        }
    }
}

impl<A, F, Req, Res> Clone for Map<A, F, Req, Res>
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

impl<A, F, Req, Res> Service<Req> for Map<A, F, Req, Res>
where
    A: Service<Req>,
    F: FnMut(A::Response) -> Res + Clone,
{
    type Response = Res;
    type Error = A::Error;
    type Future = MapFuture<A, F, Req, Res>;

    crate::forward_ready!(service);

    fn call(&self, req: Req) -> Self::Future {
        MapFuture::new(self.service.call(req), self.f.clone())
    }
}

pin_project! {
    pub struct MapFuture<A, F, Req, Res>
    where
        A: Service<Req>,
        F: FnMut(A::Response) -> Res,
    {
        f: F,
        #[pin]
        fut: A::Future,
    }
}

impl<A, F, Req, Res> MapFuture<A, F, Req, Res>
where
    A: Service<Req>,
    F: FnMut(A::Response) -> Res,
{
    fn new(fut: A::Future, f: F) -> Self {
        MapFuture { f, fut }
    }
}

impl<A, F, Req, Res> Future for MapFuture<A, F, Req, Res>
where
    A: Service<Req>,
    F: FnMut(A::Response) -> Res,
{
    type Output = Result<Res, A::Error>;

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
pub struct MapServiceFactory<A, F, Req, Res> {
    a: A,
    f: F,
    r: PhantomData<fn(Req) -> Res>,
}

impl<A, F, Req, Res> MapServiceFactory<A, F, Req, Res> {
    /// Create new `Map` new service instance
    pub(crate) fn new(a: A, f: F) -> Self
    where
        A: ServiceFactory<Req>,
        F: FnMut(A::Response) -> Res,
    {
        Self {
            a,
            f,
            r: PhantomData,
        }
    }
}

impl<A, F, Req, Res> Clone for MapServiceFactory<A, F, Req, Res>
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

impl<A, F, Req, Res> ServiceFactory<Req> for MapServiceFactory<A, F, Req, Res>
where
    A: ServiceFactory<Req>,
    F: FnMut(A::Response) -> Res + Clone,
{
    type Response = Res;
    type Error = A::Error;

    type Config = A::Config;
    type Service = Map<A::Service, F, Req, Res>;
    type InitError = A::InitError;
    type Future = MapServiceFuture<A, F, Req, Res>;

    fn new_service(&self, cfg: A::Config) -> Self::Future {
        MapServiceFuture::new(self.a.new_service(cfg), self.f.clone())
    }
}

pin_project! {
    pub struct MapServiceFuture<A, F, Req, Res>
    where
        A: ServiceFactory<Req>,
        F: FnMut(A::Response) -> Res,
    {
        #[pin]
        fut: A::Future,
        f: Option<F>,
    }
}

impl<A, F, Req, Res> MapServiceFuture<A, F, Req, Res>
where
    A: ServiceFactory<Req>,
    F: FnMut(A::Response) -> Res,
{
    fn new(fut: A::Future, f: F) -> Self {
        MapServiceFuture { f: Some(f), fut }
    }
}

impl<A, F, Req, Res> Future for MapServiceFuture<A, F, Req, Res>
where
    A: ServiceFactory<Req>,
    F: FnMut(A::Response) -> Res,
{
    type Output = Result<Map<A::Service, F, Req, Res>, A::InitError>;

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
    use futures_util::future::lazy;

    use super::*;
    use crate::{
        ok, IntoServiceFactory, Ready, Service, ServiceExt, ServiceFactory, ServiceFactoryExt,
    };

    struct Srv;

    impl Service<()> for Srv {
        type Response = ();
        type Error = ();
        type Future = Ready<Result<(), ()>>;

        crate::always_ready!();

        fn call(&self, _: ()) -> Self::Future {
            ok(())
        }
    }

    #[actix_rt::test]
    async fn test_poll_ready() {
        let srv = Srv.map(|_| "ok");
        let res = lazy(|cx| srv.poll_ready(cx)).await;
        assert_eq!(res, Poll::Ready(Ok(())));
    }

    #[actix_rt::test]
    async fn test_call() {
        let srv = Srv.map(|_| "ok");
        let res = srv.call(()).await;
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), "ok");
    }

    #[actix_rt::test]
    async fn test_new_service() {
        let new_srv = (|| ok::<_, ()>(Srv)).into_factory().map(|_| "ok");
        let srv = new_srv.new_service(&()).await.unwrap();
        let res = srv.call(()).await;
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), ("ok"));
    }
}
