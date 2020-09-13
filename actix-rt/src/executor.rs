//! This module provides a wrapper abstraction around used executor.
//! Depending on the provided feature, a corresponding set of types / helper functions is provided.
//!
//! Currently supported executors:
//!
//! - `tokio` (default)
//! - `tokio-compat`

pub use self::executor_impl::*;

#[cfg(feature = "tokio-executor")]
mod executor_impl {
    use std::io::Result;
    pub use tokio::runtime::Runtime;
    pub use tokio::task::{spawn_local, JoinHandle, LocalSet};

    pub fn build_runtime() -> Result<Runtime> {
        tokio::runtime::Builder::new()
            .enable_io()
            .enable_time()
            .basic_scheduler()
            .build()
    }

    pub fn block_on_local<F>(rt: &mut Runtime, local: &LocalSet, f: F) -> F::Output
    where
        F: std::future::Future + 'static,
    {
        local.block_on(rt, f)
    }
}

#[cfg(all(not(feature = "tokio-executor"), feature = "tokio-compat-executor"))]
mod executor_impl {
    use std::io::Result;
    pub use tokio::task::{spawn_local, JoinHandle, LocalSet};
    pub use tokio_compat::runtime::Runtime;

    pub fn build_runtime() -> Result<Runtime> {
        tokio_compat::runtime::Builder::new()
            .core_threads(1)
            .build()
    }

    pub fn block_on_local<F>(rt: &mut Runtime, local: &LocalSet, f: F) -> F::Output
    where
        F: std::future::Future + 'static,
    {
        rt.block_on_std(local.run_until(f))
    }
}
