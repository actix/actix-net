//! TCP and TLS connector services.
//!
//! # Stages of the TCP connector service:
//! 1. Resolve [`Host`] (if needed) with given [`Resolver`] and collect list of socket addresses.
//! 1. Establish TCP connection and return [`TcpStream`].
//!
//! # Stages of TLS connector services:
//! 1. Resolve DNS and establish a [`TcpStream`] with the TCP connector service.
//! 1. Wrap the stream and perform connect handshake with remote peer.
//! 1. Return wrapped stream type that implements [`AsyncRead`] and [`AsyncWrite`].
//!
//! [`TcpStream`]: actix_rt::net::TcpStream
//! [`AsyncRead`]: actix_rt::net::AsyncRead
//! [`AsyncWrite`]: actix_rt::net::AsyncWrite

mod connect_addrs;
mod connection;
mod connector;
mod error;
mod host;
mod info;
mod resolve;
mod resolver;
pub mod tcp;

#[cfg(feature = "uri")]
#[cfg_attr(docsrs, doc(cfg(feature = "uri")))]
mod uri;

#[cfg(feature = "openssl")]
#[cfg_attr(docsrs, doc(cfg(feature = "openssl")))]
pub mod openssl;

#[cfg(feature = "rustls")]
#[cfg_attr(docsrs, doc(cfg(feature = "rustls")))]
pub mod rustls;

#[cfg(feature = "native-tls")]
#[cfg_attr(docsrs, doc(cfg(feature = "native-tls")))]
pub mod native_tls;

pub use self::connection::Connection;
pub use self::connector::{Connector, ConnectorService};
pub use self::error::ConnectError;
pub use self::host::Host;
pub use self::info::ConnectionInfo;
pub use self::resolve::Resolve;
pub use self::resolver::{Resolver, ResolverService};
