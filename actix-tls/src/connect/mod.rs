//! TCP and TLS connector services.
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
//! [`TcpStream`]: actix_rt::net::TcpStream

#[allow(clippy::module_inception)]
mod connect;
mod connector;
mod error;
mod resolve;
mod service;
pub mod tls;
// TODO: remove `ssl` mod re-export in next break change
#[doc(hidden)]
pub use tls as ssl;
mod tcp;
#[cfg(feature = "uri")]
mod uri;

pub use self::connect::{Address, Connect, Connection};
pub use self::connector::{TcpConnector, TcpConnectorFactory};
pub use self::error::ConnectError;
pub use self::resolve::{Resolve, Resolver, ResolverFactory};
pub use self::service::{ConnectService, ConnectServiceFactory};
pub use self::tcp::{
    default_connector, default_connector_factory, new_connector, new_connector_factory,
};
