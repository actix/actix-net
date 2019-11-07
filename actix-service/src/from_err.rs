use std::marker::PhantomData;

use futures::{Future, Poll};

use super::{NewService, Service};
use std::pin::Pin;
use std::task::Context;

use pin_project::pin_project;

/// Service for the `from_err` combinator, changing the error type of a service.
///
/// This is created by the `ServiceExt::from_err` method.
#[pin_project]
pub struct FromErr<A, E> {
    #[pin]
    service: A,
    f: PhantomData<E>,
}

impl<A, E> FromErr<A, E> {
    pub(crate) fn new(service: A) -> Self
    where
        A: Service,
        E: From<A::Error>,
    {
        FromErr {
            service,
            f: PhantomData,
        }
    }
}

impl<A, E> Clone for FromErr<A, E>
where
    A: Clone,
{
    fn clone(&self) -> Self {
        FromErr {
            service: self.service.clone(),
            f: PhantomData,
        }
    }
}

impl<A, E> Service for FromErr<A, E>
where
    A: Service,
    E: From<A::Error>,
{
    type Request = A::Request;
    type Response = A::Response;
    type Error = E;
    type Future = FromErrFuture<A, E>;

    fn poll_ready(
        self: Pin<&mut Self>,
        ctx: &mut Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        self.project().service.poll_ready(ctx).map_err(E::from)
    }

    fn call(&mut self, req: A::Request) -> Self::Future {
        FromErrFuture {
            fut: self.service.call(req),
            f: PhantomData,
        }
    }
}

#[pin_project]
pub struct FromErrFuture<A: Service, E> {
    #[pin]
    fut: A::Future,
    f: PhantomData<E>,
}

impl<A, E> Future for FromErrFuture<A, E>
where
    A: Service,
    E: From<A::Error>,
{
    type Output = Result<A::Response, E>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.project().fut.poll(cx).map_err(E::from)
    }
}

/// NewService for the `from_err` combinator, changing the type of a new
/// service's error.
///
/// This is created by the `NewServiceExt::from_err` method.
pub struct FromErrNewService<A, E> {
    a: A,
    e: PhantomData<E>,
}

impl<A, E> FromErrNewService<A, E> {
    /// Create new `FromErr` new service instance
    pub fn new(a: A) -> Self
    where
        A: NewService,
        E: From<A::Error>,
    {
        Self { a, e: PhantomData }
    }
}

impl<A, E> Clone for FromErrNewService<A, E>
where
    A: Clone,
{
    fn clone(&self) -> Self {
        Self {
            a: self.a.clone(),
            e: PhantomData,
        }
    }
}

impl<A, E> NewService for FromErrNewService<A, E>
where
    A: NewService,
    E: From<A::Error>,
{
    type Request = A::Request;
    type Response = A::Response;
    type Error = E;

    type Config = A::Config;
    type Service = FromErr<A::Service, E>;
    type InitError = A::InitError;
    type Future = FromErrNewServiceFuture<A, E>;

    fn new_service(&self, cfg: &A::Config) -> Self::Future {
        FromErrNewServiceFuture {
            fut: self.a.new_service(cfg),
            e: PhantomData,
        }
    }
}

#[pin_project]
pub struct FromErrNewServiceFuture<A, E>
where
    A: NewService,
    E: From<A::Error>,
{
    #[pin]
    fut: A::Future,
    e: PhantomData<E>,
}

impl<A, E> Future for FromErrNewServiceFuture<A, E>
where
    A: NewService,
    E: From<A::Error>,
{
    type Output = Result<FromErr<A::Service, E>, A::InitError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if let Poll::Ready(svc) = self.project().fut.poll(cx)? {
            Poll::Ready(Ok(FromErr::new(svc)))
        } else {
            Poll::Pending
        }
    }

    /*
    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Poll::Ready(service) = self.fut.poll()? {
            Ok(Poll::Ready(FromErr::new(service)))
        } else {
            Ok(Poll::Pending)
        }
    }
    */
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

    #[derive(Debug, PartialEq)]
    struct Error;

    impl From<()> for Error {
        fn from(_: ()) -> Self {
            Error
        }
    }

    #[tokio::test]
    async fn test_poll_ready() {
        let mut srv = Srv.from_err::<Error>();
        let res = srv.poll_once().await;

        assert_eq!(res, Poll::Ready(Err(Error)));
    }

    #[tokio::test]
    async fn test_call() {
        let mut srv = Srv.from_err::<Error>();
        let res = srv.call(()).await;
        assert!(res.is_err());
        assert_eq!(res.err().unwrap(), Error);
    }

    #[tokio::test]
    async fn test_new_service() {
        let blank = || ok::<_, ()>(Srv);
        let new_srv = blank.into_new_service().from_err::<Error>();
        let mut srv = new_srv.new_service(&()).await.unwrap();
        let res = srv.call(()).await;
        assert!(res.is_err());
        assert_eq!(res.err().unwrap(), Error);
    }
}
