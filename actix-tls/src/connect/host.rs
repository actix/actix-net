//! The [`Host`] trait.

/// An interface for types where host parts (hostname and port) can be derived.
///
/// The [WHATWG URL Standard] defines the terminology used for this trait and its methods.
///
/// ```plain
/// +------------------------+
/// |          host          |
/// +-----------------+------+
/// |    hostname     | port |
/// |                 |      |
/// | sub.example.com : 8080 |
/// +-----------------+------+
/// ```
///
/// [WHATWG URL Standard]: https://url.spec.whatwg.org/
pub trait Host: Unpin + 'static {
    /// Extract hostname.
    fn hostname(&self) -> &str;

    /// Extract optional port.
    fn port(&self) -> Option<u16> {
        None
    }
}

impl Host for String {
    fn hostname(&self) -> &str {
        split_host(self).0
    }

    fn port(&self) -> Option<u16> {
        split_host(self).1.and_then(|port| port.parse().ok())
    }
}

impl Host for &'static str {
    fn hostname(&self) -> &str {
        split_host(self).0
    }

    fn port(&self) -> Option<u16> {
        split_host(self).1.and_then(|port| port.parse().ok())
    }
}

fn split_host(host: &str) -> (&str, Option<&str>) {
    if host.starts_with('[') {
        if let Some(end) = host.find(']') {
            let (hostname, rest) = host.split_at(end + 1);

            if rest.is_empty() {
                return (hostname, None);
            }

            if let Some(port) = rest.strip_prefix(':') {
                return (hostname, Some(port));
            }
        }
    }

    match host.split_once(':') {
        Some((hostname, port)) => (hostname, Some(port)),
        None => (host, None),
    }
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use actix_service::Service as _;

    use super::*;
    use crate::connect::{ConnectInfo, Resolver};

    macro_rules! assert_connection_info_eq {
        ($req:expr, $hostname:expr, $port:expr) => {{
            assert_eq!($req.hostname(), $hostname);
            assert_eq!($req.port(), $port);
        }};
    }

    #[test]
    fn host_parsing() {
        assert_connection_info_eq!("example.com", "example.com", None);
        assert_connection_info_eq!("example.com:8080", "example.com", Some(8080));
        assert_connection_info_eq!("example.com:8080".to_owned(), "example.com", Some(8080));
        assert_connection_info_eq!("example:8080", "example", Some(8080));
        assert_connection_info_eq!("example.com:false", "example.com", None);
        assert_connection_info_eq!("example.com:false:false", "example.com", None);
    }

    #[test]
    fn bracketed_ipv6_hosts() {
        assert_connection_info_eq!("[::1]:8080", "[::1]", Some(8080));
        assert_connection_info_eq!("[::1]:8080".to_owned(), "[::1]", Some(8080));

        assert_connection_info_eq!("[::1]", "[::1]", None);
        assert_connection_info_eq!("[::1]".to_owned(), "[::1]", None);
    }

    #[actix_rt::test]
    async fn bracketed_ipv6_host_resolves_to_socket_address() {
        let request = ConnectInfo::new("[::1]:8080");
        let resolved = Resolver::default().service().call(request).await.unwrap();
        let expected = "[::1]:8080".parse::<SocketAddr>().unwrap();

        assert_eq!(resolved.addrs().collect::<Vec<_>>(), vec![expected]);
    }
}
