//! TLS acceptor and connector services for Actix ecosystem

#![deny(rust_2018_idioms, nonstandard_style)]
#![doc(html_logo_url = "https://actix.rs/img/logo.png")]
#![doc(html_favicon_url = "https://actix.rs/favicon.ico")]

#[cfg(feature = "accept")]
pub mod accept;
#[cfg(feature = "connect")]
pub mod connect;
