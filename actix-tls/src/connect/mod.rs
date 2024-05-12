//! TCP and TLS connector services.
//!
//! # Stages of the TCP connector service:
//! 1. Resolve [`Host`] (if needed) with given [`Resolver`] and collect list of socket addresses.
//! 1. Establish TCP connection and return [`TcpStream`].
//!
//! # Stages of TLS connector services:
//! 1. Resolve DNS and establish a [`TcpStream`] with the TCP connector service.
//! 1. Wrap the stream and perform connect handshake with remote peer.
//! 1. Return wrapped stream type that implements `AsyncRead` and `AsyncWrite`.
//!
//! [`TcpStream`]: actix_rt::net::TcpStream

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
mod uri;

#[cfg(feature = "openssl")]
pub mod openssl;

#[cfg(any(
    feature = "rustls-0_20-webpki-roots",
    feature = "rustls-0_20-native-roots",
))]
pub mod rustls_0_20;

#[doc(hidden)]
#[cfg(any(
    feature = "rustls-0_20-webpki-roots",
    feature = "rustls-0_20-native-roots",
))]
pub use rustls_0_20 as rustls;

#[cfg(any(
    feature = "rustls-0_21-webpki-roots",
    feature = "rustls-0_21-native-roots",
))]
pub mod rustls_0_21;

#[cfg(feature = "rustls-0_22")]
pub mod rustls_0_22;

#[cfg(feature = "native-tls")]
pub mod native_tls;

pub use self::{
    connection::Connection,
    connector::{Connector, ConnectorService},
    error::ConnectError,
    host::Host,
    info::ConnectInfo,
    resolve::Resolve,
    resolver::{Resolver, ResolverService},
};
