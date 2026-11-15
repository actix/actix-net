//! Shared `/etc/resolv.conf` reader for Linux and BSD systems.
//!
//! Each call reads the file and selects the first usable `nameserver` address in file order.
//! Loopback addresses are kept so queries can use the configured local resolver.
//!
//! The parser accepts IPv4 and IPv6 addresses, whitespace, and `#` or `;` comments. It skips malformed
//! entries and addresses rejected by [`crate::usable_address`]. Scoped IPv6 addresses are skipped
//! because [`IpAddr`] cannot represent their interface scope.
//!
//! File read errors are returned unchanged. If no usable address is listed, the reader returns
//! [`io::ErrorKind::NotFound`], including when the file has no `nameserver` entries.

use std::{io, net::IpAddr};

pub(super) fn system_dns_resolver() -> io::Result<IpAddr> {
    let config = std::fs::read_to_string("/etc/resolv.conf")?;

    parse_resolv_conf(&config).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "No usable nameserver in /etc/resolv.conf",
        )
    })
}

fn parse_resolv_conf(config: &str) -> Option<IpAddr> {
    config.lines().find_map(|line| {
        let mut fields = line.split(['#', ';']).next()?.split_whitespace();

        if fields.next()? != "nameserver" {
            return None;
        }

        fields
            .next()?
            .parse()
            .ok()
            .filter(|&ip| crate::usable_address(ip))
    })
}

#[cfg(test)]
mod tests {
    use indoc::indoc;

    use super::*;

    #[test]
    fn preserves_nameserver_order_and_local_stub() {
        let config = indoc! {"
            search example.test
            nameserver 127.0.0.53
            nameserver 192.0.2.53
        "};

        assert_eq!(
            parse_resolv_conf(config),
            Some("127.0.0.53".parse().unwrap())
        );
    }

    #[test]
    fn accepts_ipv6_whitespace_and_comments() {
        let config = indoc! {"
            # nameserver 192.0.2.1\r
            ; nameserver 192.0.2.2
            \t nameserver\t2001:db8::53# comment
             nameserver 192.0.2.53 ; comment
        "};

        assert_eq!(
            parse_resolv_conf(config),
            Some("2001:db8::53".parse().unwrap())
        );
    }

    #[test]
    fn skips_malformed_entries_and_unusable_addresses() {
        let config = indoc! {"
            nameserver
            nameserver invalid
            nameservers 192.0.2.1
            nameserver 0.0.0.0
            nameserver ::
            nameserver 224.0.0.53
            nameserver ff02::1
            nameserver 255.255.255.255
            nameserver fe80::53%eth0
            nameserver fe80::53
            nameserver 192.0.2.53; comment
        "};

        assert_eq!(
            parse_resolv_conf(config),
            Some("192.0.2.53".parse().unwrap())
        );
    }

    #[test]
    fn returns_none_without_a_usable_nameserver() {
        for config in [
            "",
            "# nameserver 192.0.2.1",
            "search example.test",
            "nameserver invalid",
        ] {
            assert_eq!(parse_resolv_conf(config), None);
        }
    }
}
