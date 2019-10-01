use std::marker::PhantomData;

use futures::{Future, Poll};

use super::NewService;

use pin_project::pin_project;
use std::pin::Pin;
use std::task::Context;

/// `MapInitErr` service combinator
pub struct MapInitErr<A, F, E> {
    a: A,
    f: F,
    e: PhantomData<E>,
}

impl<A, F, E> MapInitErr<A, F, E> {
    /// Create new `MapInitErr` combinator
    pub fn new(a: A, f: F) -> Self
    where
        A: NewService,
        F: Fn(A::InitError) -> E,
    {
        Self {
            a,
            f,
            e: PhantomData,
        }
    }
}

impl<A, F, E> Clone for MapInitErr<A, F, E>
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

impl<A, F, E> NewService for MapInitErr<A, F, E>
where
    A: NewService,
    F: Fn(A::InitError) -> E + Clone,
{
    type Request = A::Request;
    type Response = A::Response;
    type Error = A::Error;

    type Config = A::Config;
    type Service = A::Service;
    type InitError = E;
    type Future = MapInitErrFuture<A, F, E>;

    fn new_service(&self, cfg: &A::Config) -> Self::Future {
        MapInitErrFuture::new(self.a.new_service(cfg), self.f.clone())
    }
}
#[pin_project]
pub struct MapInitErrFuture<A, F, E>
where
    A: NewService,
    F: Fn(A::InitError) -> E,
{
    f: F,
    #[pin]
    fut: A::Future,
}

impl<A, F, E> MapInitErrFuture<A, F, E>
where
    A: NewService,
    F: Fn(A::InitError) -> E,
{
    fn new(fut: A::Future, f: F) -> Self {
        MapInitErrFuture { f, fut }
    }
}

impl<A, F, E> Future for MapInitErrFuture<A, F, E>
where
    A: NewService,
    F: Fn(A::InitError) -> E,
{
    type Output = Result<A::Service, E>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();
        this.fut.poll(cx).map_err(this.f)
    }
}
