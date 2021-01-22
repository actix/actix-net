use std::{
    future::Future,
    net::SocketAddr,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
};

use actix_service::{Service, ServiceFactory};
use futures_core::future::LocalBoxFuture;
use log::trace;

use super::connect::{Address, Connect};
use super::error::ConnectError;

/// DNS Resolver Service factory
#[derive(Clone)]
pub struct ResolverFactory {
    resolver: Resolver,
}

impl ResolverFactory {
    pub fn new(resolver: Resolver) -> Self {
        Self { resolver }
    }

    pub fn service(&self) -> Resolver {
        self.resolver.clone()
    }
}

impl<T: Address> ServiceFactory<Connect<T>> for ResolverFactory {
    type Response = Connect<T>;
    type Error = ConnectError;
    type Config = ();
    type Service = Resolver;
    type InitError = ();
    type Future = LocalBoxFuture<'static, Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: ()) -> Self::Future {
        let service = self.resolver.clone();
        Box::pin(async { Ok(service) })
    }
}

/// DNS Resolver Service
#[derive(Clone)]
pub enum Resolver {
    Default,
    Custom(Rc<dyn Resolve>),
}

/// trait for custom lookup with self defined resolver.
///
/// # Example:
/// ```rust
/// use std::net::SocketAddr;
///
/// use actix_tls::connect::{Resolve, Resolver};
/// use futures_util::future::LocalBoxFuture;
///
/// // use trust_dns_resolver as custom resolver.
/// use trust_dns_resolver::TokioAsyncResolver;
///
/// struct MyResolver {
///     trust_dns: TokioAsyncResolver,
/// };
///
/// // impl Resolve trait and convert given host address str and port to SocketAddr.
/// impl Resolve for MyResolver {
///     fn lookup<'a>(
///         &'a self,
///         host: &'a str,
///         port: u16,
///     ) -> LocalBoxFuture<'a, Result<Vec<SocketAddr>, Box<dyn std::error::Error>>> {
///         Box::pin(async move {
///             let res = self
///                 .trust_dns
///                 .lookup_ip(host)
///                 .await?
///                 .iter()
///                 .map(|ip| SocketAddr::new(ip, port))
///                 .collect();
///             Ok(res)
///         })
///     }
/// }
///
/// let resolver = MyResolver {
///     trust_dns: TokioAsyncResolver::tokio_from_system_conf().unwrap(),
/// };
///
/// // construct custom resolver
/// let resolver = Resolver::new_custom(resolver);
///
/// // pass custom resolver to connector builder.
/// // connector would then be usable as a service or awc's connector.
/// let connector = actix_tls::connect::new_connector::<&str>(resolver.clone());
///
/// // resolver can be passed to connector factory where returned service factory
/// // can be used to construct new connector services.
/// let factory = actix_tls::connect::new_connector_factory::<&str>(resolver);
///```
pub trait Resolve {
    fn lookup<'a>(
        &'a self,
        host: &'a str,
        port: u16,
    ) -> LocalBoxFuture<'a, Result<Vec<SocketAddr>, Box<dyn std::error::Error>>>;
}

impl Resolver {
    /// Constructor for custom Resolve trait object and use it as resolver.
    pub fn new_custom(resolver: impl Resolve + 'static) -> Self {
        Self::Custom(Rc::new(resolver))
    }

    async fn lookup<T: Address>(
        &self,
        req: &Connect<T>,
    ) -> Result<Vec<SocketAddr>, ConnectError> {
        match self {
            Self::Default => {
                let host = req.host();
                // TODO: Connect should always return host with port if possible.
                let host = if req
                    .host()
                    .splitn(2, ':')
                    .last()
                    .and_then(|p| p.parse::<u16>().ok())
                    .map(|p| p == req.port())
                    .unwrap_or(false)
                {
                    host.to_string()
                } else {
                    format!("{}:{}", host, req.port())
                };

                let res = tokio::net::lookup_host(host)
                    .await
                    .map_err(|e| {
                        trace!(
                            "DNS resolver: failed to resolve host {:?} err: {}",
                            req.host(),
                            e
                        );
                        ConnectError::Resolver(Box::new(e))
                    })?
                    .collect();

                Ok(res)
            }
            Self::Custom(resolver) => resolver
                .lookup(req.host(), req.port())
                .await
                .map_err(ConnectError::Resolver),
        }
    }
}

impl<T: Address> Service<Connect<T>> for Resolver {
    type Response = Connect<T>;
    type Error = ConnectError;
    type Future = ResolverFuture<T>;

    actix_service::always_ready!();

    fn call(&mut self, req: Connect<T>) -> Self::Future {
        if !req.addr.is_none() {
            ResolverFuture::Connected(Some(req))
        } else if let Ok(ip) = req.host().parse() {
            let addr = SocketAddr::new(ip, req.port());
            let req = req.set_addr(Some(addr));
            ResolverFuture::Connected(Some(req))
        } else {
            trace!("DNS resolver: resolving host {:?}", req.host());

            let resolver = self.clone();
            ResolverFuture::Lookup(Box::pin(async move {
                let addrs = resolver.lookup(&req).await?;

                let req = req.set_addrs(addrs);

                trace!(
                    "DNS resolver: host {:?} resolved to {:?}",
                    req.host(),
                    req.addrs()
                );

                if req.addr.is_none() {
                    Err(ConnectError::NoRecords)
                } else {
                    Ok(req)
                }
            }))
        }
    }
}

pub enum ResolverFuture<T: Address> {
    Connected(Option<Connect<T>>),
    Lookup(LocalBoxFuture<'static, Result<Connect<T>, ConnectError>>),
}

impl<T: Address> Future for ResolverFuture<T> {
    type Output = Result<Connect<T>, ConnectError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.get_mut() {
            Self::Connected(conn) => Poll::Ready(Ok(conn
                .take()
                .expect("ResolverFuture polled after finished"))),
            Self::Lookup(fut) => fut.as_mut().poll(cx),
        }
    }
}
