//! The [`Resolve`] trait.

use std::{error::Error as StdError, net::SocketAddr};

use futures_core::future::LocalBoxFuture;

/// Custom async DNS resolvers.
///
/// # Examples
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
/// let my_resolver = MyResolver {
///     trust_dns: TokioAsyncResolver::tokio_from_system_conf().unwrap(),
/// };
///
/// // wrap custom resolver
/// let resolver = Resolver::custom(my_resolver);
///
/// // resolver can be passed to connector factory where returned service factory
/// // can be used to construct new connector services for use in clients
/// let factory = actix_tls::connect::Connector::new(resolver);
/// let connector = factory.service();
/// ```
pub trait Resolve {
    /// Given DNS lookup information, returns a future that completes with socket information.
    fn lookup<'a>(
        &'a self,
        host: &'a str,
        port: u16,
    ) -> LocalBoxFuture<'a, Result<Vec<SocketAddr>, Box<dyn StdError>>>;
}
