//! Tokio-based single-thread async runtime for the Actix ecosystem.

#![deny(rust_2018_idioms, nonstandard_style)]
#![allow(clippy::type_complexity)]
#![warn(missing_docs)]
#![doc(html_logo_url = "https://actix.rs/img/logo.png")]
#![doc(html_favicon_url = "https://actix.rs/favicon.ico")]

use std::future::Future;

use tokio::task::JoinHandle;

// Cannot define a main macro when compiled into test harness.
// Workaround for https://github.com/rust-lang/rust/issues/62127.
#[cfg(all(feature = "macros", not(test)))]
pub use actix_macros::{main, test};

mod builder;
mod runtime;
mod system;
mod worker;

pub use self::builder::{Builder, SystemRunner};
pub use self::runtime::Runtime;
pub use self::system::System;
pub use self::worker::Worker;

pub mod signal {
    //! Asynchronous signal handling (Tokio re-exports).

    #[cfg(unix)]
    pub mod unix {
        //! Unix specific signals (Tokio re-exports).
        pub use tokio::signal::unix::*;
    }
    pub use tokio::signal::ctrl_c;
}

pub mod net {
    //! TCP/UDP/Unix bindings (Tokio re-exports).

    pub use tokio::net::UdpSocket;
    pub use tokio::net::{TcpListener, TcpStream};

    #[cfg(unix)]
    pub use tokio::net::{UnixDatagram, UnixListener, UnixStream};
}

pub mod time {
    //! Utilities for tracking time (Tokio re-exports).

    pub use tokio::time::Instant;
    pub use tokio::time::{interval, interval_at, Interval};
    pub use tokio::time::{sleep, sleep_until, Sleep};
    pub use tokio::time::{timeout, Timeout};
}

pub mod task {
    //! Task management (Tokio re-exports).

    pub use tokio::task::{spawn_blocking, yield_now, JoinHandle};
}

/// Spawns a future on the current [Worker].
///
/// # Panics
/// Panics if Actix system is not running.
#[inline]
pub fn spawn<Fut>(f: Fut) -> JoinHandle<()>
where
    Fut: Future<Output = ()> + 'static,
{
    tokio::task::spawn_local(f)
}
