//! TLS acceptor services.

use std::sync::atomic::{AtomicUsize, Ordering};

use actix_utils::counter::Counter;

#[cfg(feature = "openssl")]
pub mod openssl;

#[cfg(feature = "rustls")]
pub mod rustls;

#[cfg(feature = "native-tls")]
pub mod native_tls;

pub(crate) static MAX_CONN: AtomicUsize = AtomicUsize::new(256);

#[cfg(any(feature = "openssl", feature = "rustls", feature = "native-tls"))]
pub(crate) const DEFAULT_TLS_HANDSHAKE_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(3);

thread_local! {
    static MAX_CONN_COUNTER: Counter = Counter::new(MAX_CONN.load(Ordering::Relaxed));
}

/// Sets the maximum per-worker concurrent TLS connection limit.
///
/// All listeners will stop accepting connections when this limit is reached.
/// It can be used to regulate the global TLS CPU usage.
///
/// By default, the connection limit is 256.
pub fn max_concurrent_tls_connect(num: usize) {
    MAX_CONN.store(num, Ordering::Relaxed);
}

/// TLS error combined with service error.
#[derive(Debug)]
pub enum TlsError<TlsErr, SvcErr> {
    Tls(TlsErr),
    Timeout,
    Service(SvcErr),
}
