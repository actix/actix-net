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

use super::{ConnectError, ConnectionInfo, Host, Resolve};

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

impl<R: Host> ServiceFactory<ConnectionInfo<R>> for Resolver {
    type Response = ConnectionInfo<R>;
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
    fn look_up<R: Host>(
        req: &ConnectionInfo<R>,
    ) -> JoinHandle<io::Result<IntoIter<SocketAddr>>> {
        let host = req.hostname();
        // TODO: Connect should always return host(name?) with port if possible; basically try to
        // reduce ability to create conflicting lookup info by having port in host string being
        // different from numeric port in connect

        let host = if req
            .hostname()
            .split_once(':')
            .and_then(|(_, port)| port.parse::<u16>().ok())
            .map(|port| port == req.port())
            .unwrap_or(false)
        {
            // if hostname contains port and also matches numeric port then just use the hostname
            host.to_string()
        } else {
            // concatenate domain-only hostname and port together
            format!("{}:{}", host, req.port())
        };

        // run blocking DNS lookup in thread pool since DNS lookups can take upwards of seconds on
        // some platforms if conditions are poor and OS-level cache is not populated
        spawn_blocking(move || std::net::ToSocketAddrs::to_socket_addrs(&host))
    }
}

impl<R: Host> Service<ConnectionInfo<R>> for ResolverService {
    type Response = ConnectionInfo<R>;
    type Error = ConnectError;
    type Future = ResolverFut<R>;

    actix_service::always_ready!();

    fn call(&self, req: ConnectionInfo<R>) -> Self::Future {
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
                    let fut = Self::look_up(&req);
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
    Connected(Option<ConnectionInfo<R>>),
    LookUp(
        JoinHandle<io::Result<IntoIter<SocketAddr>>>,
        Option<ConnectionInfo<R>>,
    ),
    LookupCustom(LocalBoxFuture<'static, Result<ConnectionInfo<R>, ConnectError>>),
}

impl<R: Host> Future for ResolverFut<R> {
    type Output = Result<ConnectionInfo<R>, ConnectError>;

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
