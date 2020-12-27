use alloc::rc::Rc;
use core::{
    cell::RefCell,
    future::Future,
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

use crate::{Service, ServiceFactory};

/// Convert `Fn(Config, &mut Service1) -> Future<Service2>` fn to a service factory.
pub fn apply_cfg<S1, Req, F, Cfg, Fut, S2, Err>(
    srv: S1,
    f: F,
) -> impl ServiceFactory<
    Req,
    Config = Cfg,
    Response = S2::Response,
    Error = S2::Error,
    Service = S2,
    InitError = Err,
    Future = Fut,
> + Clone
where
    S1: Service<Req>,
    F: FnMut(Cfg, &mut S1) -> Fut,
    Fut: Future<Output = Result<S2, Err>>,
    S2: Service<Req>,
{
    ApplyConfigService {
        srv: Rc::new(RefCell::new((srv, f))),
        _phantom: PhantomData,
    }
}

/// Convert `Fn(Config, &mut ServiceFactory1) -> Future<ServiceFactory2>` fn to a service factory.
///
/// Service1 get constructed from `T` factory.
pub fn apply_cfg_factory<SF, Req, F, Cfg, Fut, S>(
    factory: SF,
    f: F,
) -> impl ServiceFactory<
    Req,
    Config = Cfg,
    Response = S::Response,
    Error = S::Error,
    Service = S,
    InitError = SF::InitError,
> + Clone
where
    SF: ServiceFactory<Req, Config = ()>,
    F: FnMut(Cfg, &mut SF::Service) -> Fut,
    SF::InitError: From<SF::Error>,
    Fut: Future<Output = Result<S, SF::InitError>>,
    S: Service<Req>,
{
    ApplyConfigServiceFactory {
        srv: Rc::new(RefCell::new((factory, f))),
        _phantom: PhantomData,
    }
}

/// Convert `Fn(Config, &mut Server) -> Future<Service>` fn to NewService\
struct ApplyConfigService<S1, Req, F, Cfg, Fut, S2, Err>
where
    S1: Service<Req>,
    F: FnMut(Cfg, &mut S1) -> Fut,
    Fut: Future<Output = Result<S2, Err>>,
    S2: Service<Req>,
{
    srv: Rc<RefCell<(S1, F)>>,
    _phantom: PhantomData<(Cfg, Req, Fut, S2)>,
}

impl<S1, Req, F, Cfg, Fut, S2, Err> Clone for ApplyConfigService<S1, Req, F, Cfg, Fut, S2, Err>
where
    S1: Service<Req>,
    F: FnMut(Cfg, &mut S1) -> Fut,
    Fut: Future<Output = Result<S2, Err>>,
    S2: Service<Req>,
{
    fn clone(&self) -> Self {
        ApplyConfigService {
            srv: self.srv.clone(),
            _phantom: PhantomData,
        }
    }
}

impl<S1, Req, F, Cfg, Fut, S2, Err> ServiceFactory<Req>
    for ApplyConfigService<S1, Req, F, Cfg, Fut, S2, Err>
where
    S1: Service<Req>,
    F: FnMut(Cfg, &mut S1) -> Fut,
    Fut: Future<Output = Result<S2, Err>>,
    S2: Service<Req>,
{
    type Config = Cfg;
    type Response = S2::Response;
    type Error = S2::Error;
    type Service = S2;

    type InitError = Err;
    type Future = Fut;

    fn new_service(&self, cfg: Cfg) -> Self::Future {
        let (t, f) = &mut *self.srv.borrow_mut();
        f(cfg, t)
    }
}

/// Convert `Fn(&Config) -> Future<Service>` fn to NewService
struct ApplyConfigServiceFactory<SF, Req, F, Cfg, Fut, S>
where
    SF: ServiceFactory<Req, Config = ()>,
    F: FnMut(Cfg, &mut SF::Service) -> Fut,
    Fut: Future<Output = Result<S, SF::InitError>>,
    S: Service<Req>,
{
    srv: Rc<RefCell<(SF, F)>>,
    _phantom: PhantomData<(Cfg, Req, Fut, S)>,
}

impl<SF, Req, F, Cfg, Fut, S> Clone for ApplyConfigServiceFactory<SF, Req, F, Cfg, Fut, S>
where
    SF: ServiceFactory<Req, Config = ()>,
    F: FnMut(Cfg, &mut SF::Service) -> Fut,
    Fut: Future<Output = Result<S, SF::InitError>>,
    S: Service<Req>,
{
    fn clone(&self) -> Self {
        Self {
            srv: self.srv.clone(),
            _phantom: PhantomData,
        }
    }
}

impl<SF, Req, F, Cfg, Fut, S> ServiceFactory<Req>
    for ApplyConfigServiceFactory<SF, Req, F, Cfg, Fut, S>
where
    SF: ServiceFactory<Req, Config = ()>,
    SF::InitError: From<SF::Error>,
    F: FnMut(Cfg, &mut SF::Service) -> Fut,
    Fut: Future<Output = Result<S, SF::InitError>>,
    S: Service<Req>,
{
    type Config = Cfg;
    type Response = S::Response;
    type Error = S::Error;
    type Service = S;

    type InitError = SF::InitError;
    type Future = ApplyConfigServiceFactoryResponse<SF, Req, F, Cfg, Fut, S>;

    fn new_service(&self, cfg: Cfg) -> Self::Future {
        ApplyConfigServiceFactoryResponse {
            cfg: Some(cfg),
            store: self.srv.clone(),
            state: State::A(self.srv.borrow().0.new_service(())),
        }
    }
}

#[pin_project::pin_project]
struct ApplyConfigServiceFactoryResponse<SF, Req, F, Cfg, Fut, S>
where
    SF: ServiceFactory<Req, Config = ()>,
    SF::InitError: From<SF::Error>,
    F: FnMut(Cfg, &mut SF::Service) -> Fut,
    Fut: Future<Output = Result<S, SF::InitError>>,
    S: Service<Req>,
{
    cfg: Option<Cfg>,
    store: Rc<RefCell<(SF, F)>>,
    #[pin]
    state: State<SF, Fut, S, Req>,
}

#[pin_project::pin_project(project = StateProj)]
enum State<SF, Fut, S, Req>
where
    SF: ServiceFactory<Req, Config = ()>,
    SF::InitError: From<SF::Error>,
    Fut: Future<Output = Result<S, SF::InitError>>,
    S: Service<Req>,
{
    A(#[pin] SF::Future),
    B(SF::Service),
    C(#[pin] Fut),
}

impl<SF, Req, F, Cfg, Fut, S> Future
    for ApplyConfigServiceFactoryResponse<SF, Req, F, Cfg, Fut, S>
where
    SF: ServiceFactory<Req, Config = ()>,
    SF::InitError: From<SF::Error>,
    F: FnMut(Cfg, &mut SF::Service) -> Fut,
    Fut: Future<Output = Result<S, SF::InitError>>,
    S: Service<Req>,
{
    type Output = Result<S, SF::InitError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.as_mut().project();

        match this.state.as_mut().project() {
            StateProj::A(fut) => match fut.poll(cx)? {
                Poll::Pending => Poll::Pending,
                Poll::Ready(srv) => {
                    this.state.set(State::B(srv));
                    self.poll(cx)
                }
            },
            StateProj::B(srv) => match srv.poll_ready(cx)? {
                Poll::Ready(_) => {
                    {
                        let (_, f) = &mut *this.store.borrow_mut();
                        let fut = f(this.cfg.take().unwrap(), srv);
                        this.state.set(State::C(fut));
                    }
                    self.poll(cx)
                }
                Poll::Pending => Poll::Pending,
            },
            StateProj::C(fut) => fut.poll(cx),
        }
    }
}
