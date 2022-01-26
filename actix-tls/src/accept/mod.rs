//! TLS connection acceptor services.

use std::{
    convert::Infallible,
    sync::atomic::{AtomicUsize, Ordering},
};

use actix_utils::{counter::Counter, derive};

#[cfg(feature = "openssl")]
#[cfg_attr(docsrs, doc(cfg(feature = "openssl")))]
pub mod openssl;

#[cfg(feature = "rustls")]
#[cfg_attr(docsrs, doc(cfg(feature = "rustls")))]
pub mod rustls;

#[cfg(feature = "native-tls")]
#[cfg_attr(docsrs, doc(cfg(feature = "native-tls")))]
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

/// TLS handshake error, TLS timeout, or inner service error.
///
/// All TLS acceptors from this crate will return the `SvcErr` type parameter as [`Infallible`],
/// which can be cast to your own service type, inferred or otherwise,
/// using [`into_service_error`](Self::into_service_error).
#[derive(Debug)]
pub enum TlsError<TlsErr, SvcErr> {
    /// TLS handshake has timed-out.
    Timeout,
    /// Wraps TLS service errors.
    Tls(TlsErr),
    /// Wraps service errors.
    Service(SvcErr),
}

derive::enum_error! {
    match TlsError<TlsErr, SvcErr> |f| {
        Timeout => "TLS handshake has timed-out",
        Tls(e) => "TLS handshake error",
        Service(e) => "Service error",
    }
}

impl<TlsErr> TlsError<TlsErr, Infallible> {
    /// Casts the infallible service error type returned from acceptors into caller's type.
    ///
    /// # Examples
    /// ```
    /// # use std::convert::Infallible;
    /// # use actix_tls::accept::TlsError;
    /// let a: TlsError<u32, Infallible> = TlsError::Tls(42);
    /// let _b: TlsError<u32, u64> = a.into_service_error();
    /// ```
    pub fn into_service_error<SvcErr>(self) -> TlsError<TlsErr, SvcErr> {
        match self {
            Self::Timeout => TlsError::Timeout,
            Self::Tls(err) => TlsError::Tls(err),
            Self::Service(err) => match err {},
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tls_service_error_inference() {
        let a: TlsError<u32, Infallible> = TlsError::Tls(42);
        let _b: TlsError<u32, u64> = a.into_service_error();
    }
}
