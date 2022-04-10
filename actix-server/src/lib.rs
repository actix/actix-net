//! General purpose TCP server.

#![deny(rust_2018_idioms, nonstandard_style)]
#![warn(future_incompatible)]
#![doc(html_logo_url = "https://actix.rs/img/logo.png")]
#![doc(html_favicon_url = "https://actix.rs/favicon.ico")]

mod accept;
mod availability;
mod builder;
mod handle;
mod join_all;
mod server;
mod service;
mod signals;
mod socket;
mod test_server;
mod waker_queue;
mod worker;

pub use self::builder::ServerBuilder;
pub use self::handle::ServerHandle;
pub use self::server::Server;
pub use self::service::ServerServiceFactory;
#[doc(hidden)]
pub use self::socket::FromStream;
pub use self::test_server::TestServer;

/// Start server building process
#[doc(hidden)]
#[deprecated(since = "2.0.0", note = "Use `Server::build()`.")]
pub fn new() -> ServerBuilder {
    ServerBuilder::default()
}
