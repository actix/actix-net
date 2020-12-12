//! Actix utils - various helper services

#![deny(rust_2018_idioms, nonstandard_style)]
#![allow(clippy::type_complexity)]
#![doc(html_logo_url = "https://actix.rs/img/logo.png")]
#![doc(html_favicon_url = "https://actix.rs/favicon.ico")]

pub mod condition;
pub mod counter;
pub mod dispatcher;
pub mod either;
pub mod inflight;
pub mod keepalive;
pub mod mpsc;
pub mod oneshot;
pub mod order;
pub mod stream;
pub mod task;
pub mod time;
pub mod timeout;
