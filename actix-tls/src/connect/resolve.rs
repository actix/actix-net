//! [`Resolve`] trait.

use std::{error::Error as StdError, net::SocketAddr};

use futures_core::future::LocalBoxFuture;

/// Custom async DNS resolvers.
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
    ) -> LocalBoxFuture<'a, Result<Vec<SocketAddr>, Box<dyn StdError>>>;
}
