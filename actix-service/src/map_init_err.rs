use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};

use super::ServiceFactory;

/// `MapInitErr` service combinator
pub struct MapInitErr<A, F, Req, Err> {
    a: A,
    f: F,
    e: PhantomData<(Req, Err)>,
}

impl<A, Req, F, Err> MapInitErr<A, Req, F, Err>
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

impl<A, Req, F, E> ServiceFactory<Req> for MapInitErr<A, Req, F, E>
where
    A: ServiceFactory<Req>,
    F: Fn(A::InitError) -> E + Clone,
{
    type Response = A::Response;
    type Error = A::Error;

    type Config = A::Config;
    type Service = A::Service;
    type InitError = E;
    type Future = MapInitErrFuture<A, Req, F, E>;

    fn new_service(&self, cfg: A::Config) -> Self::Future {
        MapInitErrFuture::new(self.a.new_service(cfg), self.f.clone())
    }
}

#[pin_project::pin_project]
pub struct MapInitErrFuture<A, Req, F, E>
where
    A: ServiceFactory<Req>,
    F: Fn(A::InitError) -> E,
{
    f: F,
    #[pin]
    fut: A::Future,
}

impl<A, Req, F, E> MapInitErrFuture<A, Req, F, E>
where
    A: ServiceFactory<Req>,
    F: Fn(A::InitError) -> E,
{
    fn new(fut: A::Future, f: F) -> Self {
        MapInitErrFuture { f, fut }
    }
}

impl<A, Req, F, E> Future for MapInitErrFuture<A, Req, F, E>
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
