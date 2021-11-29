use std::{
    future::Future,
    io,
    net::SocketAddr,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
    vec::IntoIter,
};

use actix_rt::task::{spawn_blocking, JoinHandle};
use actix_service::{Service, ServiceFactory};
use actix_utils::future::{ok, Ready};
use futures_core::{future::LocalBoxFuture, ready};
use log::trace;

use super::{ConnectError, ConnectInfo, Host, Resolve};

/// DNS resolver service factory.
#[derive(Clone, Default)]
pub struct Resolver {
    resolver: ResolverService,
}

impl Resolver {
    /// Constructs a new resolver factory with a custom resolver.
    pub fn custom(resolver: impl Resolve + 'static) -> Self {
        Self {
            resolver: ResolverService::custom(resolver),
        }
    }

    /// Returns a new resolver service.
    pub fn service(&self) -> ResolverService {
        self.resolver.clone()
    }
}

impl<R: Host> ServiceFactory<ConnectInfo<R>> for Resolver {
    type Response = ConnectInfo<R>;
    type Error = ConnectError;
    type Config = ();
    type Service = ResolverService;
    type InitError = ();
    type Future = Ready<Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: ()) -> Self::Future {
        ok(self.resolver.clone())
    }
}

#[derive(Clone)]
enum ResolverKind {
    /// Built-in DNS resolver.
    ///
    /// See [`std::net::ToSocketAddrs`] trait.
    Default,

    /// Custom, user-provided DNS resolver.
    Custom(Rc<dyn Resolve>),
}

impl Default for ResolverKind {
    fn default() -> Self {
        Self::Default
    }
}

/// DNS resolver service.
#[derive(Clone, Default)]
pub struct ResolverService {
    kind: ResolverKind,
}

impl ResolverService {
    /// Constructor for custom Resolve trait object and use it as resolver.
    pub fn custom(resolver: impl Resolve + 'static) -> Self {
        Self {
            kind: ResolverKind::Custom(Rc::new(resolver)),
        }
    }

    /// Resolve DNS with default resolver.
    fn default_lookup<R: Host>(
        req: &ConnectInfo<R>,
    ) -> JoinHandle<io::Result<IntoIter<SocketAddr>>> {
        // reconstruct host; concatenate hostname and port together
        let host = format!("{}:{}", req.hostname(), req.port());

        // run blocking DNS lookup in thread pool since DNS lookups can take upwards of seconds on
        // some platforms if conditions are poor and OS-level cache is not populated
        spawn_blocking(move || std::net::ToSocketAddrs::to_socket_addrs(&host))
    }
}

impl<R: Host> Service<ConnectInfo<R>> for ResolverService {
    type Response = ConnectInfo<R>;
    type Error = ConnectError;
    type Future = ResolverFut<R>;

    actix_service::always_ready!();

    fn call(&self, req: ConnectInfo<R>) -> Self::Future {
        if req.addr.is_some() {
            ResolverFut::Connected(Some(req))
        } else if let Ok(ip) = req.hostname().parse() {
            let addr = SocketAddr::new(ip, req.port());
            let req = req.set_addr(Some(addr));
            ResolverFut::Connected(Some(req))
        } else {
            trace!("DNS resolver: resolving host {:?}", req.hostname());

            match &self.kind {
                ResolverKind::Default => {
                    let fut = Self::default_lookup(&req);
                    ResolverFut::LookUp(fut, Some(req))
                }

                ResolverKind::Custom(resolver) => {
                    let resolver = Rc::clone(resolver);

                    ResolverFut::LookupCustom(Box::pin(async move {
                        let addrs = resolver
                            .lookup(req.hostname(), req.port())
                            .await
                            .map_err(ConnectError::Resolver)?;

                        let req = req.set_addrs(addrs);

                        if req.addr.is_none() {
                            Err(ConnectError::NoRecords)
                        } else {
                            Ok(req)
                        }
                    }))
                }
            }
        }
    }
}

/// Future for resolver service.
pub enum ResolverFut<R: Host> {
    Connected(Option<ConnectInfo<R>>),
    LookUp(
        JoinHandle<io::Result<IntoIter<SocketAddr>>>,
        Option<ConnectInfo<R>>,
    ),
    LookupCustom(LocalBoxFuture<'static, Result<ConnectInfo<R>, ConnectError>>),
}

impl<R: Host> Future for ResolverFut<R> {
    type Output = Result<ConnectInfo<R>, ConnectError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.get_mut() {
            Self::Connected(conn) => Poll::Ready(Ok(conn
                .take()
                .expect("ResolverFuture polled after finished"))),

            Self::LookUp(fut, req) => {
                let res = match ready!(Pin::new(fut).poll(cx)) {
                    Ok(Ok(res)) => Ok(res),
                    Ok(Err(e)) => Err(ConnectError::Resolver(Box::new(e))),
                    Err(e) => Err(ConnectError::Io(e.into())),
                };

                let req = req.take().unwrap();

                let addrs = res.map_err(|err| {
                    trace!(
                        "DNS resolver: failed to resolve host {:?} err: {:?}",
                        req.hostname(),
                        err
                    );

                    err
                })?;

                let req = req.set_addrs(addrs);

                trace!(
                    "DNS resolver: host {:?} resolved to {:?}",
                    req.hostname(),
                    req.addrs()
                );

                if req.addr.is_none() {
                    Poll::Ready(Err(ConnectError::NoRecords))
                } else {
                    Poll::Ready(Ok(req))
                }
            }

            Self::LookupCustom(fut) => fut.as_mut().poll(cx),
        }
    }
}
