use core::{
    future::Future,
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

use futures_core::ready;
use pin_project_lite::pin_project;

use super::{IntoService, IntoServiceFactory, Service, ServiceFactory};

/// Apply transform function to a service.
///
/// The In and Out type params refer to the request and response types for the wrapped service.
pub fn apply_fn<I, S, F, Fut, Req, In, Res, Err>(
    service: I,
    wrap_fn: F,
) -> Apply<S, F, Req, In, Res, Err>
where
    I: IntoService<S, In>,
    S: Service<In, Error = Err>,
    F: Fn(Req, &S) -> Fut,
    Fut: Future<Output = Result<Res, Err>>,
{
    Apply::new(service.into_service(), wrap_fn)
}

/// Service factory that produces `apply_fn` service.
///
/// The In and Out type params refer to the request and response types for the wrapped service.
pub fn apply_fn_factory<I, SF, F, Fut, Req, In, Res, Err>(
    service: I,
    f: F,
) -> ApplyFactory<SF, F, Req, In, Res, Err>
where
    I: IntoServiceFactory<SF, In>,
    SF: ServiceFactory<In, Error = Err>,
    F: Fn(Req, &SF::Service) -> Fut + Clone,
    Fut: Future<Output = Result<Res, Err>>,
{
    ApplyFactory::new(service.into_factory(), f)
}

/// `Apply` service combinator.
///
/// The In and Out type params refer to the request and response types for the wrapped service.
pub struct Apply<S, F, Req, In, Res, Err>
where
    S: Service<In, Error = Err>,
{
    service: S,
    wrap_fn: F,
    _phantom: PhantomData<fn(Req) -> (In, Res, Err)>,
}

impl<S, F, Fut, Req, In, Res, Err> Apply<S, F, Req, In, Res, Err>
where
    S: Service<In, Error = Err>,
    F: Fn(Req, &S) -> Fut,
    Fut: Future<Output = Result<Res, Err>>,
{
    /// Create new `Apply` combinator
    fn new(service: S, wrap_fn: F) -> Self {
        Self {
            service,
            wrap_fn,
            _phantom: PhantomData,
        }
    }
}

impl<S, F, Fut, Req, In, Res, Err> Clone for Apply<S, F, Req, In, Res, Err>
where
    S: Service<In, Error = Err> + Clone,
    F: Fn(Req, &S) -> Fut + Clone,
    Fut: Future<Output = Result<Res, Err>>,
{
    fn clone(&self) -> Self {
        Apply {
            service: self.service.clone(),
            wrap_fn: self.wrap_fn.clone(),
            _phantom: PhantomData,
        }
    }
}

impl<S, F, Fut, Req, In, Res, Err> Service<Req> for Apply<S, F, Req, In, Res, Err>
where
    S: Service<In, Error = Err>,
    F: Fn(Req, &S) -> Fut,
    Fut: Future<Output = Result<Res, Err>>,
{
    type Response = Res;
    type Error = Err;
    type Future = Fut;

    crate::forward_ready!(service);

    fn call(&self, req: Req) -> Self::Future {
        (self.wrap_fn)(req, &self.service)
    }
}

/// `ApplyFactory` service factory combinator.
pub struct ApplyFactory<SF, F, Req, In, Res, Err> {
    factory: SF,
    wrap_fn: F,
    _phantom: PhantomData<fn(Req) -> (In, Res, Err)>,
}

impl<SF, F, Fut, Req, In, Res, Err> ApplyFactory<SF, F, Req, In, Res, Err>
where
    SF: ServiceFactory<In, Error = Err>,
    F: Fn(Req, &SF::Service) -> Fut + Clone,
    Fut: Future<Output = Result<Res, Err>>,
{
    /// Create new `ApplyFactory` new service instance
    fn new(factory: SF, wrap_fn: F) -> Self {
        Self {
            factory,
            wrap_fn,
            _phantom: PhantomData,
        }
    }
}

impl<SF, F, Fut, Req, In, Res, Err> Clone for ApplyFactory<SF, F, Req, In, Res, Err>
where
    SF: ServiceFactory<In, Error = Err> + Clone,
    F: Fn(Req, &SF::Service) -> Fut + Clone,
    Fut: Future<Output = Result<Res, Err>>,
{
    fn clone(&self) -> Self {
        Self {
            factory: self.factory.clone(),
            wrap_fn: self.wrap_fn.clone(),
            _phantom: PhantomData,
        }
    }
}

impl<SF, F, Fut, Req, In, Res, Err> ServiceFactory<Req>
    for ApplyFactory<SF, F, Req, In, Res, Err>
where
    SF: ServiceFactory<In, Error = Err>,
    F: Fn(Req, &SF::Service) -> Fut + Clone,
    Fut: Future<Output = Result<Res, Err>>,
{
    type Response = Res;
    type Error = Err;

    type Config = SF::Config;
    type Service = Apply<SF::Service, F, Req, In, Res, Err>;
    type InitError = SF::InitError;
    type Future = ApplyServiceFactoryResponse<SF, F, Fut, Req, In, Res, Err>;

    fn new_service(&self, cfg: SF::Config) -> Self::Future {
        let svc = self.factory.new_service(cfg);
        ApplyServiceFactoryResponse::new(svc, self.wrap_fn.clone())
    }
}

pin_project! {
    pub struct ApplyServiceFactoryResponse<SF, F, Fut, Req, In, Res, Err>
    where
        SF: ServiceFactory<In, Error = Err>,
        F: Fn(Req, &SF::Service) -> Fut,
        Fut: Future<Output = Result<Res, Err>>,
    {
        #[pin]
        fut: SF::Future,
        wrap_fn: Option<F>,
        _phantom: PhantomData<fn(Req) -> Res>,
    }
}

impl<SF, F, Fut, Req, In, Res, Err> ApplyServiceFactoryResponse<SF, F, Fut, Req, In, Res, Err>
where
    SF: ServiceFactory<In, Error = Err>,
    F: Fn(Req, &SF::Service) -> Fut,
    Fut: Future<Output = Result<Res, Err>>,
{
    fn new(fut: SF::Future, wrap_fn: F) -> Self {
        Self {
            fut,
            wrap_fn: Some(wrap_fn),
            _phantom: PhantomData,
        }
    }
}

impl<SF, F, Fut, Req, In, Res, Err> Future
    for ApplyServiceFactoryResponse<SF, F, Fut, Req, In, Res, Err>
where
    SF: ServiceFactory<In, Error = Err>,
    F: Fn(Req, &SF::Service) -> Fut,
    Fut: Future<Output = Result<Res, Err>>,
{
    type Output = Result<Apply<SF::Service, F, Req, In, Res, Err>, SF::InitError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        let svc = ready!(this.fut.poll(cx))?;
        Poll::Ready(Ok(Apply::new(svc, this.wrap_fn.take().unwrap())))
    }
}

#[cfg(test)]
mod tests {
    use core::task::Poll;

    use futures_util::future::lazy;

    use super::*;
    use crate::{
        ok,
        pipeline::{pipeline, pipeline_factory},
        Ready, Service, ServiceFactory,
    };

    #[derive(Clone)]
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
    async fn test_call() {
        let srv = pipeline(apply_fn(Srv, |req: &'static str, srv| {
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

        let srv = new_srv.new_service(()).await.unwrap();

        assert_eq!(lazy(|cx| srv.poll_ready(cx)).await, Poll::Ready(Ok(())));

        let res = srv.call("srv").await;
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), ("srv", ()));
    }
}
