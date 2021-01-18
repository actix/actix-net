use std::{net::SocketAddr, rc::Rc, task::Poll};

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
/// let connector = actix_tls::connect::new_connector(resolver.clone());
///
/// // resolver can be passed to connector factory where returned service factory
/// // can be used to construct new connector services.
/// let factory = actix_tls::connect::new_connector_factory(resolver);
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
                let host = if req.host().contains(':') {
                    req.host().to_string()
                } else {
                    format!("{}:{}", req.host(), req.port())
                };

                let res = tokio::net::lookup_host(host).await.map_err(|e| {
                    trace!(
                        "DNS resolver: failed to resolve host {:?} err: {}",
                        req.host(),
                        e
                    );
                    ConnectError::Resolver(Box::new(e))
                })?;

                Ok(res.collect())
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
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    actix_service::always_ready!();

    fn call(&mut self, mut req: Connect<T>) -> Self::Future {
        let resolver = self.clone();
        Box::pin(async move {
            if req.addr.is_some() {
                Ok(req)
            } else if let Ok(ip) = req.host().parse() {
                req.addr = Some(either::Either::Left(SocketAddr::new(ip, req.port())));
                Ok(req)
            } else {
                trace!("DNS resolver: resolving host {:?}", req.host());

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
            }
        })
    }
}
