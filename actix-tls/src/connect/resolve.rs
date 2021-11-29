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
use futures_core::{future::LocalBoxFuture, ready};
use log::trace;

use super::connect::{Address, Connect};
use super::error::ConnectError;

/// DNS resolver service factory.
#[derive(Clone)]
pub struct ResolverFactory {
    resolver: Resolver,
}

impl ResolverFactory {
    /// Constructs a new resolver factory with the given resolver.
    pub fn new(resolver: Resolver) -> Self {
        Self { resolver }
    }

    /// Returns a reference to the inner resolver.
    pub fn service(&self) -> Resolver {
        self.resolver.clone()
    }
}

impl<R: Address> ServiceFactory<Connect<R>> for ResolverFactory {
    type Response = Connect<R>;
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

/// An interface for custom async DNS resolvers.
///
/// # Usage
/// ```
/// use std::net::SocketAddr;
///
/// use actix_tls::connect::{Resolve, Resolver};
/// use futures_util::future::LocalBoxFuture;
///
/// // use trust-dns async tokio resolver
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
/// // connector would then be usable as a service or an `awc` connector.
/// let connector = actix_tls::connect::new_connector::<&str>(resolver.clone());
///
/// // resolver can be passed to connector factory where returned service factory
/// // can be used to construct new connector services.
/// let factory = actix_tls::connect::new_connector_factory::<&str>(resolver);
/// ```
pub trait Resolve {
    /// Given DNS lookup information, returns a future that completes with socket information.
    fn lookup<'a>(
        &'a self,
        host: &'a str,
        port: u16,
    ) -> LocalBoxFuture<'a, Result<Vec<SocketAddr>, Box<dyn std::error::Error>>>;
}

/// DNS resolver service
#[derive(Clone)]
pub enum Resolver {
    /// Built-in DNS resolver.
    ///
    /// See [`std::net::ToSocketAddrs`] trait.
    Default,

    /// Custom, user-provided DNS resolver.
    Custom(Rc<dyn Resolve>),
}

impl Default for Resolver {
    fn default() -> Self {
        Self::Default
    }
}

impl Resolver {
    /// Constructor for custom Resolve trait object and use it as resolver.
    pub fn new_custom(resolver: impl Resolve + 'static) -> Self {
        Self::Custom(Rc::new(resolver))
    }

    /// Resolve DNS with default resolver.
    fn look_up<R: Address>(req: &Connect<R>) -> JoinHandle<io::Result<IntoIter<SocketAddr>>> {
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

impl<R: Address> Service<Connect<R>> for Resolver {
    type Response = Connect<R>;
    type Error = ConnectError;
    type Future = ResolverFuture<R>;

    actix_service::always_ready!();

    fn call(&self, req: Connect<R>) -> Self::Future {
        if req.addr.is_some() {
            ResolverFuture::Connected(Some(req))
        } else if let Ok(ip) = req.hostname().parse() {
            let addr = SocketAddr::new(ip, req.port());
            let req = req.set_addr(Some(addr));
            ResolverFuture::Connected(Some(req))
        } else {
            trace!("DNS resolver: resolving host {:?}", req.hostname());

            match self {
                Self::Default => {
                    let fut = Self::look_up(&req);
                    ResolverFuture::LookUp(fut, Some(req))
                }

                Self::Custom(resolver) => {
                    let resolver = Rc::clone(resolver);
                    ResolverFuture::LookupCustom(Box::pin(async move {
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

pub enum ResolverFuture<R: Address> {
    Connected(Option<Connect<R>>),
    LookUp(
        JoinHandle<io::Result<IntoIter<SocketAddr>>>,
        Option<Connect<R>>,
    ),
    LookupCustom(LocalBoxFuture<'static, Result<Connect<R>, ConnectError>>),
}

impl<R: Address> Future for ResolverFuture<R> {
    type Output = Result<Connect<R>, ConnectError>;

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
