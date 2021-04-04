//! General purpose TCP server.

#![deny(rust_2018_idioms, nonstandard_style)]
#![doc(html_logo_url = "https://actix.rs/img/logo.png")]
#![doc(html_favicon_url = "https://actix.rs/favicon.ico")]

mod accept;
mod builder;
mod config;
mod server;
mod service;
mod signals;
mod socket;
mod test_server;
mod waker_queue;
mod worker;

pub use self::builder::ServerBuilder;
pub use self::config::{ServiceConfig, ServiceRuntime};
pub use self::server::{Server, ServerHandle};
pub use self::service::ServiceFactory;
pub use self::test_server::TestServer;

#[doc(hidden)]
pub use self::socket::FromStream;

/// Socket ID token
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Token(usize);

impl Default for Token {
    fn default() -> Self {
        Self::new()
    }
}

impl Token {
    fn new() -> Self {
        Self(0)
    }

    pub(crate) fn next(&mut self) -> Token {
        let token = Token(self.0);
        self.0 += 1;
        token
    }
}

/// Start server building process
pub fn new() -> ServerBuilder {
    ServerBuilder::default()
}
