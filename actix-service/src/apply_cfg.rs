use alloc::rc::Rc;
use core::{
    future::Future,
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

use futures_core::ready;
use pin_project_lite::pin_project;

use crate::{Service, ServiceFactory};

/// Convert `Fn(Config, &Service1) -> Future<Service2>` fn to a service factory.
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
    F: Fn(Cfg, &S1) -> Fut,
    Fut: Future<Output = Result<S2, Err>>,
    S2: Service<Req>,
{
    ApplyConfigService {
        srv: Rc::new((srv, f)),
        _phantom: PhantomData,
    }
}

/// Convert `Fn(Config, &ServiceFactory1) -> Future<ServiceFactory2>` fn to a service factory.
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
    F: Fn(Cfg, &SF::Service) -> Fut,
    SF::InitError: From<SF::Error>,
    Fut: Future<Output = Result<S, SF::InitError>>,
    S: Service<Req>,
{
    ApplyConfigServiceFactory {
        srv: Rc::new((factory, f)),
        _phantom: PhantomData,
    }
}

/// Convert `Fn(Config, &Server) -> Future<Service>` fn to NewService\
struct ApplyConfigService<S1, Req, F, Cfg, Fut, S2, Err>
where
    S1: Service<Req>,
    F: Fn(Cfg, &S1) -> Fut,
    Fut: Future<Output = Result<S2, Err>>,
    S2: Service<Req>,
{
    srv: Rc<(S1, F)>,
    _phantom: PhantomData<(Cfg, Req, Fut, S2)>,
}

impl<S1, Req, F, Cfg, Fut, S2, Err> Clone for ApplyConfigService<S1, Req, F, Cfg, Fut, S2, Err>
where
    S1: Service<Req>,
    F: Fn(Cfg, &S1) -> Fut,
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
    F: Fn(Cfg, &S1) -> Fut,
    Fut: Future<Output = Result<S2, Err>>,
    S2: Service<Req>,
{
    type Response = S2::Response;
    type Error = S2::Error;
    type Config = Cfg;
    type Service = S2;

    type InitError = Err;
    type Future = Fut;

    fn new_service(&self, cfg: Cfg) -> Self::Future {
        let (t, f) = &*self.srv;
        f(cfg, t)
    }
}

/// Convert `Fn(&Config) -> Future<Service>` fn to NewService
struct ApplyConfigServiceFactory<SF, Req, F, Cfg, Fut, S>
where
    SF: ServiceFactory<Req, Config = ()>,
    F: Fn(Cfg, &SF::Service) -> Fut,
    Fut: Future<Output = Result<S, SF::InitError>>,
    S: Service<Req>,
{
    srv: Rc<(SF, F)>,
    _phantom: PhantomData<(Cfg, Req, Fut, S)>,
}

impl<SF, Req, F, Cfg, Fut, S> Clone for ApplyConfigServiceFactory<SF, Req, F, Cfg, Fut, S>
where
    SF: ServiceFactory<Req, Config = ()>,
    F: Fn(Cfg, &SF::Service) -> Fut,
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
    F: Fn(Cfg, &SF::Service) -> Fut,
    Fut: Future<Output = Result<S, SF::InitError>>,
    S: Service<Req>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Config = Cfg;
    type Service = S;

    type InitError = SF::InitError;
    type Future = ApplyConfigServiceFactoryResponse<SF, Req, F, Cfg, Fut, S>;

    fn new_service(&self, cfg: Cfg) -> Self::Future {
        ApplyConfigServiceFactoryResponse {
            cfg: Some(cfg),
            store: self.srv.clone(),
            state: State::A {
                fut: self.srv.0.new_service(()),
            },
        }
    }
}

pin_project! {
    struct ApplyConfigServiceFactoryResponse<SF, Req, F, Cfg, Fut, S>
    where
        SF: ServiceFactory<Req, Config = ()>,
        SF::InitError: From<SF::Error>,
        F: Fn(Cfg, &SF::Service) -> Fut,
        Fut: Future<Output = Result<S, SF::InitError>>,
        S: Service<Req>,
    {
        cfg: Option<Cfg>,
        store: Rc<(SF, F)>,
        #[pin]
        state: State<SF, Fut, S, Req>,
    }
}

pin_project! {
    #[project = StateProj]
    enum State<SF, Fut, S, Req>
    where
        SF: ServiceFactory<Req, Config = ()>,
        SF::InitError: From<SF::Error>,
        Fut: Future<Output = Result<S, SF::InitError>>,
        S: Service<Req>,
    {
        A { #[pin] fut: SF::Future },
        B { svc: SF::Service },
        C { #[pin] fut: Fut },
    }
}

impl<SF, Req, F, Cfg, Fut, S> Future
    for ApplyConfigServiceFactoryResponse<SF, Req, F, Cfg, Fut, S>
where
    SF: ServiceFactory<Req, Config = ()>,
    SF::InitError: From<SF::Error>,
    F: Fn(Cfg, &SF::Service) -> Fut,
    Fut: Future<Output = Result<S, SF::InitError>>,
    S: Service<Req>,
{
    type Output = Result<S, SF::InitError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.as_mut().project();

        match this.state.as_mut().project() {
            StateProj::A { fut } => {
                let svc = ready!(fut.poll(cx))?;
                this.state.set(State::B { svc });
                self.poll(cx)
            }
            StateProj::B { svc } => {
                ready!(svc.poll_ready(cx))?;
                {
                    let (_, f) = &**this.store;
                    let fut = f(this.cfg.take().unwrap(), svc);
                    this.state.set(State::C { fut });
                }
                self.poll(cx)
            }
            StateProj::C { fut } => fut.poll(cx),
        }
    }
}
