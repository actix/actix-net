use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};

use super::{IntoService, IntoServiceFactory, Service, ServiceFactory};

/// Apply transform function to a service.
pub fn apply_fn<T, Req, F, R, In, Out, Err, U>(
    service: U,
    f: F,
) -> Apply<T, Req, F, R, In, Out, Err>
where
    T: Service<Req, Error = Err>,
    F: FnMut(In, &mut T) -> R,
    R: Future<Output = Result<Out, Err>>,
    U: IntoService<T, Req>,
{
    Apply::new(service.into_service(), f)
}

/// Service factory that produces `apply_fn` service.
pub fn apply_fn_factory<T, Req, F, R, In, Out, Err, U>(
    service: U,
    f: F,
) -> ApplyServiceFactory<T, Req, F, R, In, Out, Err>
where
    T: ServiceFactory<Req, Error = Err>,
    F: FnMut(In, &mut T::Service) -> R + Clone,
    R: Future<Output = Result<Out, Err>>,
    U: IntoServiceFactory<T, Req>,
{
    ApplyServiceFactory::new(service.into_factory(), f)
}

/// `Apply` service combinator
pub struct Apply<T, Req, F, R, In, Out, Err>
where
    T: Service<Req, Error = Err>,
{
    service: T,
    f: F,
    r: PhantomData<(In, Out, R)>,
}

impl<T, Req, F, R, In, Out, Err> Apply<T, Req, F, R, In, Out, Err>
where
    T: Service<Req, Error = Err>,
    F: FnMut(In, &mut T) -> R,
    R: Future<Output = Result<Out, Err>>,
{
    /// Create new `Apply` combinator
    fn new(service: T, f: F) -> Self {
        Self {
            service,
            f,
            r: PhantomData,
        }
    }
}

impl<T, Req, F, R, In, Out, Err> Clone for Apply<T, Req, F, R, In, Out, Err>
where
    T: Service<Req, Error = Err> + Clone,
    F: FnMut(In, &mut T) -> R + Clone,
    R: Future<Output = Result<Out, Err>>,
{
    fn clone(&self) -> Self {
        Apply {
            service: self.service.clone(),
            f: self.f.clone(),
            r: PhantomData,
        }
    }
}

impl<T, Req, F, R, In, Out, Err> Service<Req> for Apply<T, Req, F, R, In, Out, Err>
where
    T: Service<Req, Error = Err>,
    F: FnMut(In, &mut T) -> R,
    R: Future<Output = Result<Out, Err>>,
{
    // type Request = In;
    type Response = Out;
    type Error = Err;
    type Future = R;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(futures_util::ready!(self.service.poll_ready(cx)))
    }

    fn call(&mut self, req: In) -> Self::Future {
        (self.f)(req, &mut self.service)
    }
}

/// `apply()` service factory
pub struct ApplyServiceFactory<T, Req, F, R, In, Out, Err>
where
    T: ServiceFactory<Req, Error = Err>,
    F: FnMut(In, &mut T::Service) -> R + Clone,
    R: Future<Output = Result<Out, Err>>,
{
    service: T,
    f: F,
    r: PhantomData<(R, In, Out)>,
}

impl<T, Req, F, R, In, Out, Err> ApplyServiceFactory<T, Req, F, R, In, Out, Err>
where
    T: ServiceFactory<Req, Error = Err>,
    F: FnMut(In, &mut T::Service) -> R + Clone,
    R: Future<Output = Result<Out, Err>>,
{
    /// Create new `ApplyNewService` new service instance
    fn new(service: T, f: F) -> Self {
        Self {
            f,
            service,
            r: PhantomData,
        }
    }
}

impl<T, Req, F, R, In, Out, Err> Clone for ApplyServiceFactory<T, Req, F, R, In, Out, Err>
where
    T: ServiceFactory<Req, Error = Err> + Clone,
    F: FnMut(In, &mut T::Service) -> R + Clone,
    R: Future<Output = Result<Out, Err>>,
{
    fn clone(&self) -> Self {
        Self {
            service: self.service.clone(),
            f: self.f.clone(),
            r: PhantomData,
        }
    }
}

impl<T, Req, F, R, In, Out, Err> ServiceFactory<Req>
    for ApplyServiceFactory<T, Req, F, R, In, Out, Err>
where
    T: ServiceFactory<Req, Error = Err>,
    F: FnMut(In, &mut T::Service) -> R + Clone,
    R: Future<Output = Result<Out, Err>>,
{
    // type Request = In;
    type Response = Out;
    type Error = Err;

    type Config = T::Config;
    type Service = Apply<T::Service, Req, F, R, In, Out, Err>;
    type InitError = T::InitError;
    type Future = ApplyServiceFactoryResponse<T,Req, F, R, In, Out, Err>;

    fn new_service(&self, cfg: T::Config) -> Self::Future {
        ApplyServiceFactoryResponse::new(self.service.new_service(cfg), self.f.clone())
    }
}

#[pin_project::pin_project]
pub struct ApplyServiceFactoryResponse<T, Req, F, R, In, Out, Err>
where
    T: ServiceFactory<Req, Error = Err>,
    F: FnMut(In, &mut T::Service) -> R,
    R: Future<Output = Result<Out, Err>>,
{
    #[pin]
    fut: T::Future,
    f: Option<F>,
    r: PhantomData<(In, Out)>,
}

impl<T, Req, F, R, In, Out, Err> ApplyServiceFactoryResponse<T, Req, F, R, In, Out, Err>
where
    T: ServiceFactory<Req, Error = Err>,
    F: FnMut(In, &mut T::Service) -> R,
    R: Future<Output = Result<Out, Err>>,
{
    fn new(fut: T::Future, f: F) -> Self {
        Self {
            f: Some(f),
            fut,
            r: PhantomData,
        }
    }
}

impl<T, Req, F, R, In, Out, Err> Future
    for ApplyServiceFactoryResponse<T, Req, F, R, In, Out, Err>
where
    T: ServiceFactory<Req, Error = Err>,
    F: FnMut(In, &mut T::Service) -> R,
    R: Future<Output = Result<Out, Err>>,
{
    type Output = Result<Apply<T::Service, Req, F, R, In, Out, Err>, T::InitError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
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
    use std::task::{Context, Poll};

    use futures_util::future::{lazy, ok, Ready};

    use super::*;
    use crate::{pipeline, pipeline_factory, Service, ServiceFactory};

    #[derive(Clone)]
    struct Srv;

    impl Service<()> for Srv {
        type Response = ();
        type Error = ();
        type Future = Ready<Result<(), ()>>;

        fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, _: ()) -> Self::Future {
            ok(())
        }
    }

    #[actix_rt::test]
    async fn test_call() {
        let mut srv = pipeline(apply_fn(Srv, |req: &'static str, srv| {
            let fut = srv.call(());
            async move {
                fut.await.unwrap();
                Ok((req, ()))
            }
        }));

        assert_eq!(lazy(|cx| srv.poll_ready(cx)).await, Poll::Ready(Ok(())));

        let res = srv.call("srv").await;
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), ("srv", ()));
    }

    #[actix_rt::test]
    async fn test_new_service() {
        let new_srv = pipeline_factory(apply_fn_factory(
            || ok::<_, ()>(Srv),
            |req: &'static str, srv| {
                let fut = srv.call(());
                async move {
                    fut.await.unwrap();
                    Ok((req, ()))
                }
            },
        ));

        let mut srv = new_srv.new_service(()).await.unwrap();

        assert_eq!(lazy(|cx| srv.poll_ready(cx)).await, Poll::Ready(Ok(())));

        let res = srv.call("srv").await;
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), ("srv", ()));
    }
}
