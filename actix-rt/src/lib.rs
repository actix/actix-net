//! A runtime implementation that runs everything on the current thread.

#[cfg(not(test))] // Work around for rust-lang/rust#62127
pub use actix_macros::{main, test};

mod arbiter;
mod system;

pub use self::arbiter::Arbiter;
pub use self::system::System;

#[doc(hidden)]
pub use actix_threadpool as blocking;

/// Spawns a future on the current arbiter.
///
/// # Panics
///
/// This function panics if actix system is not running.
pub fn spawn<F>(f: F)
where
    F: futures::Future<Output = ()> + 'static,
{
    if !System::is_set() {
        panic!("System is not running");
    }

    Arbiter::spawn(f);
}

/// Utilities for tracking time.
pub mod time {
    pub use tokio::time::{Duration, Instant};
    pub use tokio::time::{delay_for, delay_until, Delay};
    pub use tokio::time::{interval, interval_at, Interval};
    pub use tokio::time::{timeout, Timeout};
}
