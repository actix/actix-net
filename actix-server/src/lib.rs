//! General purpose TCP server.

#![deny(rust_2018_idioms, nonstandard_style)]
#![doc(html_logo_url = "https://actix.rs/img/logo.png")]
#![doc(html_favicon_url = "https://actix.rs/favicon.ico")]

mod accept;
mod builder;
mod server;
mod service;
mod signals;
mod socket;
mod test_server;
mod waker_queue;
mod worker;

pub use self::builder::ServerBuilder;
pub use self::server::{Server, ServerHandle};
pub use self::service::ServiceFactory;
pub use self::test_server::TestServer;

#[doc(hidden)]
pub use self::socket::FromStream;

/// Start server building process
pub fn new() -> ServerBuilder {
    ServerBuilder::default()
}
