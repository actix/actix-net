//! SSL Services
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::counter::Counter;

#[cfg(feature = "ssl")]
mod openssl;
#[cfg(feature = "ssl")]
pub use self::openssl::OpensslAcceptor;

#[cfg(feature = "tls")]
mod nativetls;
#[cfg(feature = "tls")]
pub use self::nativetls::NativeTlsAcceptor;

#[cfg(feature = "rust-tls")]
mod rustls;
#[cfg(feature = "rust-tls")]
pub use self::rustls::RustlsAcceptor;

/// Sets the maximum per-worker concurrent ssl connection establish process.
///
/// All listeners will stop accepting connections when this limit is
/// reached. It can be used to limit the global SSL CPU usage.
///
/// By default max connections is set to a 256.
pub fn max_concurrent_ssl_connect(num: usize) {
    MAX_CONN.store(num, Ordering::Relaxed);
}

pub(crate) static MAX_CONN: AtomicUsize = AtomicUsize::new(256);

thread_local! {
    static MAX_CONN_COUNTER: Counter = Counter::new(MAX_CONN.load(Ordering::Relaxed));
}

/// Ssl error combinded with service error.
#[derive(Debug)]
pub enum SslError<E1, E2> {
    Ssl(E1),
    Service(E2),
}
