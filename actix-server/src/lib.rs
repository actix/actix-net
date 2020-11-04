//! General purpose TCP server.

#![deny(rust_2018_idioms)]

mod accept;
mod builder;
mod config;
mod server;
mod service;
mod signals;
mod socket;
mod waker_queue;
mod worker;

pub use self::builder::ServerBuilder;
pub use self::config::{ServiceConfig, ServiceRuntime};
pub use self::server::Server;
pub use self::service::ServiceFactory;

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

pub(crate) type LocalBoxFuture<'a, T> =
    std::pin::Pin<Box<dyn std::future::Future<Output = T> + 'a>>;

/// Start server building process
pub fn new() -> ServerBuilder {
    ServerBuilder::default()
}
