//! Actix utils - various helper services

#![deny(rust_2018_idioms, nonstandard_style)]
#![allow(clippy::type_complexity)]
#![doc(html_logo_url = "https://actix.rs/img/logo.png")]
#![doc(html_favicon_url = "https://actix.rs/favicon.ico")]

pub mod counter;
pub mod dispatcher;
pub mod mpsc;
pub mod task;
pub mod timeout;

#[deprecated(since = "2.1.0", note = "actix_utils::condition has been removed")]
pub mod condition {}
#[deprecated(since = "2.1.0", note = "actix_utils::either has been removed")]
pub mod either {}
#[deprecated(since = "2.1.0", note = "actix_utils::inflight has been removed")]
pub mod inflight {}
#[deprecated(since = "2.1.0", note = "actix_utils::keepalive has been removed")]
pub mod keepalive {}
#[deprecated(since = "2.1.0", note = "actix_utils::oneshot has been removed")]
pub mod oneshot {}
#[deprecated(since = "2.1.0", note = "actix_utils::order has been removed")]
pub mod order {}
#[deprecated(since = "2.1.0", note = "actix_utils::stream has been removed")]
pub mod stream {}
#[deprecated(since = "2.1.0", note = "actix_utils::time has been removed")]
pub mod time {}
