use pin_project::pin_project;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};

use super::{IntoService, IntoServiceFactory, Service, ServiceFactory};

/// Apply tranform function to a service
pub fn apply_fn<T, F, R, In, Out, Err, U>(
    service: U,
    f: F,
) -> impl Service<Request = In, Response = Out, Error = Err>
where
    T: Service<Error = Err>,
    F: FnMut(In, &mut T) -> R,
    R: Future<Output = Result<Out, Err>>,
    U: IntoService<T>,
{
    Apply::new(service.into_service(), f)
}

/// Create factory for `apply` service.
pub fn apply_fn_factory<T, F, R, In, Out, Err, U>(
    service: U,
    f: F,
) -> impl ServiceFactory<
    Config = T::Config,
    Request = In,
    Response = Out,
    Error = Err,
    InitError = T::InitError,
>
where
    T: ServiceFactory<Error = Err>,
    F: FnMut(In, &mut T::Service) -> R + Clone,
    R: Future<Output = Result<Out, Err>>,
    U: IntoServiceFactory<T>,
{
    ApplyNewService::new(service.into_factory(), f)
}

#[doc(hidden)]
/// `Apply` service combinator
#[pin_project]
struct Apply<T, F, R, In, Out, Err>
where
    T: Service<Error = Err>,
{
    #[pin]
    service: T,
    f: F,
    r: PhantomData<(In, Out, R)>,
}

impl<T, F, R, In, Out, Err> Apply<T, F, R, In, Out, Err>
where
    T: Service<Error = Err>,
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

impl<T, F, R, In, Out, Err> Service for Apply<T, F, R, In, Out, Err>
where
    T: Service<Error = Err>,
    F: FnMut(In, &mut T) -> R,
    R: Future<Output = Result<Out, Err>>,
{
    type Request = In;
    type Response = Out;
    type Error = Err;
    type Future = R;

    fn poll_ready(&mut self, ctx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(futures::ready!(self.service.poll_ready(ctx)))
    }

    fn call(&mut self, req: In) -> Self::Future {
        (self.f)(req, &mut self.service)
    }
}

/// `ApplyNewService` new service combinator
struct ApplyNewService<T, F, R, In, Out, Err>
where
    T: ServiceFactory<Error = Err>,
{
    service: T,
    f: F,
    r: PhantomData<(R, In, Out)>,
}

impl<T, F, R, In, Out, Err> ApplyNewService<T, F, R, In, Out, Err>
where
    T: ServiceFactory<Error = Err>,
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

impl<T, F, R, In, Out, Err> ServiceFactory for ApplyNewService<T, F, R, In, Out, Err>
where
    T: ServiceFactory<Error = Err>,
    F: FnMut(In, &mut T::Service) -> R + Clone,
    R: Future<Output = Result<Out, Err>>,
{
    type Request = In;
    type Response = Out;
    type Error = Err;

    type Config = T::Config;
    type Service = Apply<T::Service, F, R, In, Out, Err>;
    type InitError = T::InitError;
    type Future = ApplyNewServiceFuture<T, F, R, In, Out, Err>;

    fn new_service(&self, cfg: &T::Config) -> Self::Future {
        ApplyNewServiceFuture::new(self.service.new_service(cfg), self.f.clone())
    }
}

#[pin_project]
struct ApplyNewServiceFuture<T, F, R, In, Out, Err>
where
    T: ServiceFactory<Error = Err>,
    F: FnMut(In, &mut T::Service) -> R + Clone,
    R: Future<Output = Result<Out, Err>>,
{
    #[pin]
    fut: T::Future,
    f: Option<F>,
    r: PhantomData<(In, Out)>,
}

impl<T, F, R, In, Out, Err> ApplyNewServiceFuture<T, F, R, In, Out, Err>
where
    T: ServiceFactory<Error = Err>,
    F: FnMut(In, &mut T::Service) -> R + Clone,
    R: Future<Output = Result<Out, Err>>,
{
    fn new(fut: T::Future, f: F) -> Self {
        ApplyNewServiceFuture {
            f: Some(f),
            fut,
            r: PhantomData,
        }
    }
}

impl<T, F, R, In, Out, Err> Future for ApplyNewServiceFuture<T, F, R, In, Out, Err>
where
    T: ServiceFactory<Error = Err>,
    F: FnMut(In, &mut T::Service) -> R + Clone,
    R: Future<Output = Result<Out, Err>>,
{
    type Output = Result<Apply<T::Service, F, R, In, Out, Err>, T::InitError>;

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

    use futures::future::{lazy, ok, Ready};

    use super::*;
    use crate::{pipeline, pipeline_factory, Service, ServiceFactory};

    #[derive(Clone)]
    struct Srv;

    impl Service for Srv {
        type Request = ();
        type Response = ();
        type Error = ();
        type Future = Ready<Result<(), ()>>;

        fn poll_ready(&mut self, _: &mut Context) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, _: ()) -> Self::Future {
            ok(())
        }
    }

    #[tokio::test]
    async fn test_call() {
        let mut srv = pipeline(apply_fn(Srv, |req: &'static str, srv| {
            let fut = srv.call(());
            async move {
                let res = fut.await.unwrap();
                Ok((req, res))
            }
        }));

        assert_eq!(lazy(|cx| srv.poll_ready(cx)).await, Poll::Ready(Ok(())));

        let res = srv.call("srv").await;
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), (("srv", ())));
    }

    #[tokio::test]
    async fn test_new_service() {
        let new_srv = pipeline_factory(apply_fn_factory(
            || ok::<_, ()>(Srv),
            |req: &'static str, srv| {
                let fut = srv.call(());
                async move {
                    let res = fut.await.unwrap();
                    Ok((req, res))
                }
            },
        ));

        let mut srv = new_srv.new_service(&()).await.unwrap();

        assert_eq!(lazy(|cx| srv.poll_ready(cx)).await, Poll::Ready(Ok(())));

        let res = srv.call("srv").await;
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), (("srv", ())));
    }
}
