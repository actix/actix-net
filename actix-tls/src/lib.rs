//! TLS acceptor and connector services for the Actix ecosystem.

#![deny(rust_2018_idioms, nonstandard_style)]
#![warn(missing_docs)]
#![doc(html_logo_url = "https://actix.rs/img/logo.png")]
#![doc(html_favicon_url = "https://actix.rs/favicon.ico")]
#![cfg_attr(docsrs, feature(doc_cfg))]

#[cfg(feature = "openssl")]
#[allow(unused_extern_crates)]
extern crate tls_openssl as openssl;

#[cfg(feature = "accept")]
#[cfg_attr(docsrs, doc(cfg(feature = "accept")))]
pub mod accept;

#[cfg(feature = "connect")]
#[cfg_attr(docsrs, doc(cfg(feature = "connect")))]
pub mod connect;
