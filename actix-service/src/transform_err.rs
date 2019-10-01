use std::marker::PhantomData;

use futures::{Future, Poll};

use super::Transform;
use std::pin::Pin;
use std::task::Context;

use pin_project::pin_project;

/// Transform for the `map_err` combinator, changing the type of a new
/// transform's init error.
///
/// This is created by the `Transform::map_err` method.
pub struct TransformMapInitErr<T, S, F, E> {
    t: T,
    f: F,
    e: PhantomData<(S, E)>,
}

impl<T, S, F, E> TransformMapInitErr<T, S, F, E> {
    /// Create new `TransformMapErr` new transform instance
    pub fn new(t: T, f: F) -> Self
    where
        T: Transform<S>,
        F: Fn(T::InitError) -> E,
    {
        Self {
            t,
            f,
            e: PhantomData,
        }
    }
}

impl<T, S, F, E> Clone for TransformMapInitErr<T, S, F, E>
where
    T: Clone,
    F: Clone,
{
    fn clone(&self) -> Self {
        Self {
            t: self.t.clone(),
            f: self.f.clone(),
            e: PhantomData,
        }
    }
}

impl<T, S, F, E> Transform<S> for TransformMapInitErr<T, S, F, E>
where
    T: Transform<S>,
    F: Fn(T::InitError) -> E + Clone,
{
    type Request = T::Request;
    type Response = T::Response;
    type Error = T::Error;
    type Transform = T::Transform;

    type InitError = E;
    type Future = TransformMapInitErrFuture<T, S, F, E>;

    fn new_transform(&self, service: S) -> Self::Future {
        TransformMapInitErrFuture {
            fut: self.t.new_transform(service),
            f: self.f.clone(),
        }
    }
}
#[pin_project]
pub struct TransformMapInitErrFuture<T, S, F, E>
where
    T: Transform<S>,
    F: Fn(T::InitError) -> E,
{
    #[pin]
    fut: T::Future,
    f: F,
}

impl<T, S, F, E> Future for TransformMapInitErrFuture<T, S, F, E>
where
    T: Transform<S>,
    F: Fn(T::InitError) -> E + Clone,
{
    type Output = Result<T::Transform, E>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        this.fut.poll(cx).map_err(this.f)
    }
}

/// Transform for the `from_err` combinator, changing the type of a new
/// transform's init error.
///
/// This is created by the `Transform::from_err` method.
pub struct TransformFromErr<T, S, E> {
    t: T,
    e: PhantomData<(S, E)>,
}

impl<T, S, E> TransformFromErr<T, S, E>
where
    T: Transform<S>,
    E: From<T::InitError>,
{
    /// Create new `TransformFromErr` new transform instance
    pub fn new(t: T) -> Self {
        Self { t, e: PhantomData }
    }
}

impl<T, S, E> Clone for TransformFromErr<T, S, E>
where
    T: Clone,
{
    fn clone(&self) -> Self {
        Self {
            t: self.t.clone(),
            e: PhantomData,
        }
    }
}

impl<T, S, E> Transform<S> for TransformFromErr<T, S, E>
where
    T: Transform<S>,
    E: From<T::InitError>,
{
    type Request = T::Request;
    type Response = T::Response;
    type Error = T::Error;
    type Transform = T::Transform;

    type InitError = E;
    type Future = TransformFromErrFuture<T, S, E>;

    fn new_transform(&self, service: S) -> Self::Future {
        TransformFromErrFuture {
            fut: self.t.new_transform(service),
            _t: PhantomData,
        }
    }
}

#[pin_project]
pub struct TransformFromErrFuture<T, S, E>
where
    T: Transform<S>,
    E: From<T::InitError>,
{
    #[pin]
    fut: T::Future,
    _t: PhantomData<E>,
}

impl<T, S, E> Future for TransformFromErrFuture<T, S, E>
where
    T: Transform<S>,
    E: From<T::InitError>,
{
    type Output = Result<T::Transform, E>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.project().fut.poll(cx).map_err(E::from)
    }
}
