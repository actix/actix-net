use core::{
    future::Future,
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

use pin_project_lite::pin_project;

use super::ServiceFactory;

/// `MapInitErr` service combinator
pub struct MapInitErr<A, F, Req, Err> {
    a: A,
    f: F,
    e: PhantomData<fn(Req) -> Err>,
}

impl<A, F, Req, Err> MapInitErr<A, F, Req, Err>
where
    A: ServiceFactory<Req>,
    F: Fn(A::InitError) -> Err,
{
    /// Create new `MapInitErr` combinator
    pub(crate) fn new(a: A, f: F) -> Self {
        Self {
            a,
            f,
            e: PhantomData,
        }
    }
}

impl<A, F, Req, E> Clone for MapInitErr<A, F, Req, E>
where
    A: Clone,
    F: Clone,
{
    fn clone(&self) -> Self {
        Self {
            a: self.a.clone(),
            f: self.f.clone(),
            e: PhantomData,
        }
    }
}

impl<A, F, Req, E> ServiceFactory<Req> for MapInitErr<A, F, Req, E>
where
    A: ServiceFactory<Req>,
    F: Fn(A::InitError) -> E + Clone,
{
    type Response = A::Response;
    type Error = A::Error;

    type Config = A::Config;
    type Service = A::Service;
    type InitError = E;
    type Future = MapInitErrFuture<A, F, Req, E>;

    fn new_service(&self, cfg: A::Config) -> Self::Future {
        MapInitErrFuture::new(self.a.new_service(cfg), self.f.clone())
    }
}

pin_project! {
    pub struct MapInitErrFuture<A, F, Req, E>
    where
        A: ServiceFactory<Req>,
        F: Fn(A::InitError) -> E,
    {
        f: F,
        #[pin]
        fut: A::Future,
    }
}

impl<A, F, Req, E> MapInitErrFuture<A, F, Req, E>
where
    A: ServiceFactory<Req>,
    F: Fn(A::InitError) -> E,
{
    fn new(fut: A::Future, f: F) -> Self {
        MapInitErrFuture { f, fut }
    }
}

impl<A, F, Req, E> Future for MapInitErrFuture<A, F, Req, E>
where
    A: ServiceFactory<Req>,
    F: Fn(A::InitError) -> E,
{
    type Output = Result<A::Service, E>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        this.fut.poll(cx).map_err(this.f)
    }
}
