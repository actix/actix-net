//! TCP connector services for Actix ecosystem.
//!
//! # Stages of the TCP connector service:
//! - Resolve [`Address`] with given [`Resolver`] and collect list of socket addresses.
//! - Establish TCP connection and return [`TcpStream`].
//!
//! # Stages of TLS connector services:
//! - Establish [`TcpStream`] with connector service.
//! - Wrap the stream and perform connect handshake with remote peer.
//! - Return certain stream type that impls `AsyncRead` and `AsyncWrite`.
//!
//! # Package feature
//! * `openssl` - enables TLS support via `openssl` crate
//! * `rustls` - enables TLS support via `rustls` crate
//!
//! [`TcpStream`]: actix_rt::net::TcpStream

#[allow(clippy::module_inception)]
mod connect;
mod connector;
mod error;
mod resolve;
mod service;
pub mod ssl;
#[cfg(feature = "uri")]
mod uri;

use actix_rt::net::TcpStream;
use actix_service::{Service, ServiceFactory};

pub use self::connect::{Address, Connect, Connection};
pub use self::connector::{TcpConnector, TcpConnectorFactory};
pub use self::error::ConnectError;
pub use self::resolve::{Resolve, Resolver, ResolverFactory};
pub use self::service::{ConnectService, ConnectServiceFactory};

/// Create TCP connector service.
pub fn new_connector<T: Address + 'static>(
    resolver: Resolver,
) -> impl Service<Connect<T>, Response = Connection<T, TcpStream>, Error = ConnectError> + Clone
{
    ConnectServiceFactory::new(resolver).service()
}

/// Create TCP connector service factory.
pub fn new_connector_factory<T: Address + 'static>(
    resolver: Resolver,
) -> impl ServiceFactory<
    Connect<T>,
    Config = (),
    Response = Connection<T, TcpStream>,
    Error = ConnectError,
    InitError = (),
> + Clone {
    ConnectServiceFactory::new(resolver)
}

/// Create connector service with default parameters.
pub fn default_connector<T: Address + 'static>(
) -> impl Service<Connect<T>, Response = Connection<T, TcpStream>, Error = ConnectError> + Clone
{
    new_connector(Resolver::Default)
}

/// Create connector service factory with default parameters.
pub fn default_connector_factory<T: Address + 'static>() -> impl ServiceFactory<
    Connect<T>,
    Config = (),
    Response = Connection<T, TcpStream>,
    Error = ConnectError,
    InitError = (),
> + Clone {
    new_connector_factory(Resolver::Default)
}
