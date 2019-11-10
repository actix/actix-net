use std::marker::PhantomData;

use futures::{ready, Future, Poll};

use super::{IntoNewService, IntoService, NewService, Service};

use crate::IntoFuture;
use pin_project::pin_project;
use std::pin::Pin;
use std::task::Context;

/// Apply tranform function to a service
pub fn apply_fn<T, F, In, Out, U>(service: U, f: F) -> Apply<T, F, In, Out>
where
    T: Service,
    F: FnMut(In, &mut T) -> Out,
    Out: IntoFuture,
    Out::Error: From<T::Error>,
    U: IntoService<T>,
{
    Apply::new(service.into_service(), f)
}

/// Create factory for `apply` service.
pub fn new_apply_fn<T, F, In, Out, U>(service: U, f: F) -> ApplyNewService<T, F, In, Out>
where
    T: NewService,
    F: FnMut(In, &mut T::Service) -> Out + Clone,
    Out: IntoFuture,
    Out::Error: From<T::Error>,
    U: IntoNewService<T>,
{
    ApplyNewService::new(service.into_new_service(), f)
}

#[doc(hidden)]
/// `Apply` service combinator
#[pin_project]
pub struct Apply<T, F, In, Out>
where
    T: Service,
{
    #[pin]
    service: T,
    f: F,
    r: PhantomData<(In, Out)>,
}

impl<T, F, In, Out> Apply<T, F, In, Out>
where
    T: Service,
    F: FnMut(In, &mut T) -> Out,
    Out: IntoFuture,
    Out::Error: From<T::Error>,
{
    /// Create new `Apply` combinator
    pub(crate) fn new<I: IntoService<T>>(service: I, f: F) -> Self {
        Self {
            service: service.into_service(),
            f,
            r: PhantomData,
        }
    }
}

impl<T, F, In, Out> Clone for Apply<T, F, In, Out>
where
    T: Service + Clone,
    F: Clone,
{
    fn clone(&self) -> Self {
        Apply {
            service: self.service.clone(),
            f: self.f.clone(),
            r: PhantomData,
        }
    }
}

impl<T, F, In, Out> Service for Apply<T, F, In, Out>
where
    T: Service,
    F: FnMut(In, &mut T) -> Out,
    Out: IntoFuture,
    Out::Error: From<T::Error>,
{
    type Request = In;
    type Response = Out::Item;
    type Error = Out::Error;
    type Future = Out::Future;

    fn poll_ready(
        mut self: Pin<&mut Self>,
        ctx: &mut Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(ready!(self.project().service.poll_ready(ctx)).map_err(|e| e.into()))
    }

    fn call(&mut self, req: In) -> Self::Future {
        (self.f)(req, &mut self.service).into_future()
    }
}

/// `ApplyNewService` new service combinator
pub struct ApplyNewService<T, F, In, Out>
where
    T: NewService,
{
    service: T,
    f: F,
    r: PhantomData<(In, Out)>,
}

impl<T, F, In, Out> ApplyNewService<T, F, In, Out>
where
    T: NewService,
    F: FnMut(In, &mut T::Service) -> Out + Clone,
    Out: IntoFuture,
    Out::Error: From<T::Error>,
{
    /// Create new `ApplyNewService` new service instance
    pub(crate) fn new<F1: IntoNewService<T>>(service: F1, f: F) -> Self {
        Self {
            f,
            service: service.into_new_service(),
            r: PhantomData,
        }
    }
}

impl<T, F, In, Out> Clone for ApplyNewService<T, F, In, Out>
where
    T: NewService + Clone,
    F: FnMut(In, &mut T::Service) -> Out + Clone,
    Out: IntoFuture,
{
    fn clone(&self) -> Self {
        Self {
            service: self.service.clone(),
            f: self.f.clone(),
            r: PhantomData,
        }
    }
}

impl<T, F, In, Out> NewService for ApplyNewService<T, F, In, Out>
where
    T: NewService,
    F: FnMut(In, &mut T::Service) -> Out + Clone,
    Out: IntoFuture,
    Out::Error: From<T::Error>,
{
    type Request = In;
    type Response = Out::Item;
    type Error = Out::Error;

    type Config = T::Config;
    type Service = Apply<T::Service, F, In, Out>;
    type InitError = T::InitError;
    type Future = ApplyNewServiceFuture<T, F, In, Out>;

    fn new_service(&self, cfg: &T::Config) -> Self::Future {
        ApplyNewServiceFuture::new(self.service.new_service(cfg), self.f.clone())
    }
}

#[pin_project]
pub struct ApplyNewServiceFuture<T, F, In, Out>
where
    T: NewService,
    F: FnMut(In, &mut T::Service) -> Out + Clone,
    Out: IntoFuture,
{
    #[pin]
    fut: T::Future,
    f: Option<F>,
    r: PhantomData<(In, Out)>,
}

impl<T, F, In, Out> ApplyNewServiceFuture<T, F, In, Out>
where
    T: NewService,
    F: FnMut(In, &mut T::Service) -> Out + Clone,
    Out: IntoFuture,
{
    fn new(fut: T::Future, f: F) -> Self {
        ApplyNewServiceFuture {
            f: Some(f),
            fut,
            r: PhantomData,
        }
    }
}

impl<T, F, In, Out> Future for ApplyNewServiceFuture<T, F, In, Out>
where
    T: NewService,
    F: FnMut(In, &mut T::Service) -> Out + Clone,
    Out: IntoFuture,
    Out::Error: From<T::Error>,
{
    type Output = Result<Apply<T::Service, F, In, Out>, T::InitError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        if let Poll::Ready(svc) = this.fut.poll(cx)? {
            Poll::Ready(Ok(Apply::new(svc, this.f.take().unwrap())))
        } else {
            Poll::Pending
        }
    }
}

#[cfg(test)]
mod tests {
    use futures::future::{ok, Ready};
    use futures::{Future, Poll, TryFutureExt};

    use super::*;
    use crate::{IntoService, NewService, Service, ServiceExt};

    #[derive(Clone)]
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
    async fn test_call() {
        let blank = |req| ok(req);

        let mut srv = blank
            .into_service()
            .apply_fn(Srv, |req: &'static str, srv| {
                srv.call(()).map_ok(move |res| (req, res))
            });
        assert_eq!(srv.poll_once().await, Poll::Ready(Ok(())));
        let res = srv.call("srv").await;
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), (("srv", ())));
    }

    #[tokio::test]
    async fn test_new_service() {
        let new_srv = ApplyNewService::new(
            || ok::<_, ()>(Srv),
            |req: &'static str, srv| srv.call(()).map_ok(move |res| (req, res)),
        );

        let mut srv = new_srv.new_service(&()).await.unwrap();

        assert_eq!(srv.poll_once().await, Poll::Ready(Ok(())));
        let res = srv.call("srv").await;
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), (("srv", ())));
    }
}
