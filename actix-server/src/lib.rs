//! General purpose TCP server.
//!
//! # Tracing
//!
//! Each dispatched connection has a root `connection` span at the `DEBUG` level, with
//! `worker_id`, `connection_id`, `service`, and `local_addr` fields. The local address identifies
//! the bound listener (a TCP address or Unix socket address). Connection IDs are local to each worker
//! instance. The span covers the service call and polling of its returned future. Spans created
//! by the service can use it as their parent. Tasks spawned by the service must propagate the
//! current span explicitly.

#![doc(html_logo_url = "https://actix.rs/img/logo.png")]
#![doc(html_favicon_url = "https://actix.rs/favicon.ico")]

mod accept;
mod availability;
mod builder;
mod handle;
mod join_all;
mod server;
mod service;
mod shutdown;
mod signals;
mod socket;
mod test_server;
mod waker_queue;
mod worker;

#[doc(hidden)]
pub use self::socket::FromStream;
pub use self::{
    builder::{MpTcp, ServerBuilder},
    handle::ServerHandle,
    server::Server,
    service::ServerServiceFactory,
    shutdown::GracefulShutdownSignal,
    test_server::TestServer,
};

/// Start server building process
#[doc(hidden)]
#[deprecated(since = "2.0.0", note = "Use `Server::build()`.")]
pub fn new() -> ServerBuilder {
    ServerBuilder::default()
}
