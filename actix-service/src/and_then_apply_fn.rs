use std::cell::RefCell;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

use crate::{Service, ServiceFactory};

/// `Apply` service combinator
pub(crate) struct AndThenApplyFn<S1, S2, F, Fut, Req, In, Res, Err>
where
    S1: Service<Req>,
    S2: Service<In>,
    F: FnMut(S1::Response, &mut S2) -> Fut,
    Fut: Future<Output = Result<Res, Err>>,
    Err: From<S1::Error> + From<S2::Error>,
{
    svc: Rc<RefCell<(S1, S2, F)>>,
    _phantom: PhantomData<(Fut, Req, In, Res, Err)>,
}

impl<S1, S2, F, Fut, Req, In, Res, Err> AndThenApplyFn<S1, S2, F, Fut, Req, In, Res, Err>
where
    S1: Service<Req>,
    S2: Service<In>,
    F: FnMut(S1::Response, &mut S2) -> Fut,
    Fut: Future<Output = Result<Res, Err>>,
    Err: From<S1::Error> + From<S2::Error>,
{
    /// Create new `Apply` combinator
    pub(crate) fn new(a: S1, b: S2, wrap_fn: F) -> Self {
        Self {
            svc: Rc::new(RefCell::new((a, b, wrap_fn))),
            _phantom: PhantomData,
        }
    }
}

impl<S1, S2, F, Fut, Req, In, Res, Err> Clone
    for AndThenApplyFn<S1, S2, F, Fut, Req, In, Res, Err>
where
    S1: Service<Req>,
    S2: Service<In>,
    F: FnMut(S1::Response, &mut S2) -> Fut,
    Fut: Future<Output = Result<Res, Err>>,
    Err: From<S1::Error> + From<S2::Error>,
{
    fn clone(&self) -> Self {
        AndThenApplyFn {
            svc: self.svc.clone(),
            _phantom: PhantomData,
        }
    }
}

impl<S1, S2, F, Fut, Req, In, Res, Err> Service<Req>
    for AndThenApplyFn<S1, S2, F, Fut, Req, In, Res, Err>
where
    S1: Service<Req>,
    S2: Service<In>,
    F: FnMut(S1::Response, &mut S2) -> Fut,
    Fut: Future<Output = Result<Res, Err>>,
    Err: From<S1::Error> + From<S2::Error>,
{
    type Response = Res;
    type Error = Err;
    type Future = AndThenApplyFnFuture<S1, S2, F, Fut, Req, In, Res, Err>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let mut inner = self.svc.borrow_mut();
        let not_ready = inner.0.poll_ready(cx)?.is_pending();
        if inner.1.poll_ready(cx)?.is_pending() || not_ready {
            Poll::Pending
        } else {
            Poll::Ready(Ok(()))
        }
    }

    fn call(&mut self, req: Req) -> Self::Future {
        let fut = self.svc.borrow_mut().0.call(req);
        AndThenApplyFnFuture {
            state: State::A(fut, Some(self.svc.clone())),
        }
    }
}

#[pin_project::pin_project]
pub(crate) struct AndThenApplyFnFuture<S1, S2, F, Fut, Req, In, Res, Err>
where
    S1: Service<Req>,
    S2: Service<In>,
    F: FnMut(S1::Response, &mut S2) -> Fut,
    Fut: Future<Output = Result<Res, Err>>,
    Err: From<S1::Error> + From<S2::Error>,
{
    #[pin]
    state: State<S1, S2, F, Fut, Req, In, Res, Err>,
}

#[pin_project::pin_project(project = StateProj)]
enum State<S1, S2, F, Fut, Req, In, Res, Err>
where
    S1: Service<Req>,
    S2: Service<In>,
    F: FnMut(S1::Response, &mut S2) -> Fut,
    Fut: Future<Output = Result<Res, Err>>,
    Err: From<S1::Error> + From<S2::Error>,
{
    A(#[pin] S1::Future, Option<Rc<RefCell<(S1, S2, F)>>>),
    B(#[pin] Fut),
    Empty(PhantomData<In>),
}

impl<S1, S2, F, Fut, Req, In, Res, Err> Future
    for AndThenApplyFnFuture<S1, S2, F, Fut, Req, In, Res, Err>
where
    S1: Service<Req>,
    S2: Service<In>,
    F: FnMut(S1::Response, &mut S2) -> Fut,
    Fut: Future<Output = Result<Res, Err>>,
    Err: From<S1::Error> + From<S2::Error>,
{
    type Output = Result<Res, Err>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.as_mut().project();

        match this.state.as_mut().project() {
            StateProj::A(fut, b) => match fut.poll(cx)? {
                Poll::Ready(res) => {
                    let b = Option::take(b).unwrap();
                    this.state.set(State::Empty(PhantomData));
                    let (_, b, f) = &mut *b.borrow_mut();
                    let fut = f(res, b);
                    this.state.set(State::B(fut));
                    self.poll(cx)
                }
                Poll::Pending => Poll::Pending,
            },
            StateProj::B(fut) => fut.poll(cx).map(|r| {
                this.state.set(State::Empty(PhantomData));
                r
            }),
            StateProj::Empty(_) => {
                panic!("future must not be polled after it returned `Poll::Ready`")
            }
        }
    }
}

/// `AndThenApplyFn` service factory
pub(crate) struct AndThenApplyFnFactory<SF1, SF2, F, Fut, Req, In, Res, Err> {
    srv: Rc<(SF1, SF2, F)>,
    _phantom: PhantomData<(Fut, Req, In, Res, Err)>,
}

impl<SF1, SF2, F, Fut, Req, In, Res, Err>
    AndThenApplyFnFactory<SF1, SF2, F, Fut, Req, In, Res, Err>
where
    SF1: ServiceFactory<Req>,
    SF2: ServiceFactory<In, Config = SF1::Config, InitError = SF1::InitError>,
    F: FnMut(SF1::Response, &mut SF2::Service) -> Fut + Clone,
    Fut: Future<Output = Result<Res, Err>>,
    Err: From<SF1::Error> + From<SF2::Error>,
{
    /// Create new `ApplyNewService` new service instance
    pub(crate) fn new(a: SF1, b: SF2, wrap_fn: F) -> Self {
        Self {
            srv: Rc::new((a, b, wrap_fn)),
            _phantom: PhantomData,
        }
    }
}

impl<SF1, SF2, F, Fut, Req, In, Res, Err> Clone
    for AndThenApplyFnFactory<SF1, SF2, F, Fut, Req, In, Res, Err>
{
    fn clone(&self) -> Self {
        Self {
            srv: self.srv.clone(),
            _phantom: PhantomData,
        }
    }
}

impl<SF1, SF2, F, Fut, Req, In, Res, Err> ServiceFactory<Req>
    for AndThenApplyFnFactory<SF1, SF2, F, Fut, Req, In, Res, Err>
where
    SF1: ServiceFactory<Req>,
    SF1::Config: Clone,
    SF2: ServiceFactory<In, Config = SF1::Config, InitError = SF1::InitError>,
    F: FnMut(SF1::Response, &mut SF2::Service) -> Fut + Clone,
    Fut: Future<Output = Result<Res, Err>>,
    Err: From<SF1::Error> + From<SF2::Error>,
{
    type Response = Res;
    type Error = Err;
    type Service = AndThenApplyFn<SF1::Service, SF2::Service, F, Fut, Req, In, Res, Err>;
    type Config = SF1::Config;
    type InitError = SF1::InitError;
    type Future = AndThenApplyFnFactoryResponse<SF1, SF2, F, Fut, Req, In, Res, Err>;

    fn new_service(&self, cfg: SF1::Config) -> Self::Future {
        let srv = &*self.srv;
        AndThenApplyFnFactoryResponse {
            s1: None,
            s2: None,
            wrap_fn: srv.2.clone(),
            fut_s1: srv.0.new_service(cfg.clone()),
            fut_s2: srv.1.new_service(cfg),
            _phantom: PhantomData,
        }
    }
}

#[pin_project::pin_project]
pub(crate) struct AndThenApplyFnFactoryResponse<SF1, SF2, F, Fut, Req, In, Res, Err>
where
    SF1: ServiceFactory<Req>,
    SF2: ServiceFactory<In, Config = SF1::Config, InitError = SF1::InitError>,
    F: FnMut(SF1::Response, &mut SF2::Service) -> Fut + Clone,
    Fut: Future<Output = Result<Res, Err>>,
    Err: From<SF1::Error>,
    Err: From<SF2::Error>,
{
    #[pin]
    fut_s1: SF1::Future,
    #[pin]
    fut_s2: SF2::Future,
    wrap_fn: F,
    s1: Option<SF1::Service>,
    s2: Option<SF2::Service>,
    _phantom: PhantomData<In>,
}

impl<SF1, SF2, F, Fut, Req, In, Res, Err> Future
    for AndThenApplyFnFactoryResponse<SF1, SF2, F, Fut, Req, In, Res, Err>
where
    SF1: ServiceFactory<Req>,
    SF2: ServiceFactory<In, Config = SF1::Config, InitError = SF1::InitError>,
    F: FnMut(SF1::Response, &mut SF2::Service) -> Fut + Clone,
    Fut: Future<Output = Result<Res, Err>>,
    Err: From<SF1::Error> + From<SF2::Error>,
{
    type Output = Result<
        AndThenApplyFn<SF1::Service, SF2::Service, F, Fut, Req, In, Res, Err>,
        SF1::InitError,
    >;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        if this.s1.is_none() {
            if let Poll::Ready(service) = this.fut_s1.poll(cx)? {
                *this.s1 = Some(service);
            }
        }

        if this.s2.is_none() {
            if let Poll::Ready(service) = this.fut_s2.poll(cx)? {
                *this.s2 = Some(service);
            }
        }

        if this.s1.is_some() && this.s2.is_some() {
            Poll::Ready(Ok(AndThenApplyFn {
                svc: Rc::new(RefCell::new((
                    Option::take(this.s1).unwrap(),
                    Option::take(this.s2).unwrap(),
                    this.wrap_fn.clone(),
                ))),
                _phantom: PhantomData,
            }))
        } else {
            Poll::Pending
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use futures_util::future::{lazy, ok, Ready, TryFutureExt};

    use crate::{fn_service, pipeline, pipeline_factory, Service, ServiceFactory};

    #[derive(Clone)]
    struct Srv;

    impl Service<u8> for Srv {
        type Response = ();
        type Error = ();
        type Future = Ready<Result<Self::Response, Self::Error>>;

        fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, req: u8) -> Self::Future {
            let _ = req;
            ok(())
        }
    }

    #[actix_rt::test]
    async fn test_service() {
        let mut srv = pipeline(ok).and_then_apply_fn(Srv, |req: &'static str, s| {
            s.call(1).map_ok(move |res| (req, res))
        });
        let res = lazy(|cx| srv.poll_ready(cx)).await;
        assert!(res.is_ready());

        let res = srv.call("srv").await;
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), ("srv", ()));
    }

    #[actix_rt::test]
    async fn test_service_factory() {
        let new_srv = pipeline_factory(|| ok::<_, ()>(fn_service(ok))).and_then_apply_fn(
            || ok(Srv),
            |req: &'static str, s| s.call(1).map_ok(move |res| (req, res)),
        );
        let mut srv = new_srv.new_service(()).await.unwrap();
        let res = lazy(|cx| srv.poll_ready(cx)).await;
        assert!(res.is_ready());

        let res = srv.call("srv").await;
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), ("srv", ()));
    }
}
