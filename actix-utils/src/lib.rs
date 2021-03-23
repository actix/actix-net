//! Various network related services and utilities for the Actix ecosystem.

#![deny(rust_2018_idioms, nonstandard_style)]
#![allow(clippy::type_complexity)]
#![doc(html_logo_url = "https://actix.rs/img/logo.png")]
#![doc(html_favicon_url = "https://actix.rs/favicon.ico")]

pub mod counter;
pub mod dispatcher;
pub mod mpsc;
mod poll_fn;
pub mod task;
pub mod timeout;

use self::poll_fn::poll_fn;
