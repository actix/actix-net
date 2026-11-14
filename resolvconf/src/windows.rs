use std::net::{IpAddr, Ipv6Addr};

pub(super) fn configured_dns_resolver() -> std::io::Result<IpAddr> {
    let adapters = ipconfig::get_adapters().map_err(std::io::Error::other)?;

    first_resolver(adapters.iter().map(|adapter| {
        (
            adapter.oper_status() == ipconfig::OperStatus::IfOperStatusUp,
            adapter.dns_servers(),
        )
    }))
    .ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no usable DNS server on an operational adapter",
        )
    })
}

fn first_resolver<'a>(adapters: impl IntoIterator<Item = (bool, &'a [IpAddr])>) -> Option<IpAddr> {
    adapters
        .into_iter()
        .filter(|(up, _)| *up)
        .flat_map(|(_, servers)| servers.iter().copied())
        .find(|&ip| crate::usable_address(ip) && !OBSOLETE_SERVERS.contains(&ip))
}

// These site-local placeholders are not configured DNS server endpoints.
const OBSOLETE_SERVERS: [IpAddr; 3] = [
    IpAddr::V6(Ipv6Addr::new(0xfec0, 0, 0, 0xffff, 0, 0, 0, 1)),
    IpAddr::V6(Ipv6Addr::new(0xfec0, 0, 0, 0xffff, 0, 0, 0, 2)),
    IpAddr::V6(Ipv6Addr::new(0xfec0, 0, 0, 0xffff, 0, 0, 0, 3)),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_down_and_empty_adapters() {
        let down = ["192.0.2.1".parse().unwrap()];
        let up = ["192.0.2.53".parse().unwrap()];

        assert_eq!(
            first_resolver([(false, down.as_slice()), (true, &[]), (true, up.as_slice())]),
            Some(up[0]),
        );
    }

    #[test]
    fn preserves_adapter_and_server_order() {
        let first = [
            "2001:db8::53".parse().unwrap(),
            "192.0.2.53".parse().unwrap(),
        ];
        let second = ["192.0.2.54".parse().unwrap()];

        assert_eq!(
            first_resolver([(true, first.as_slice()), (true, second.as_slice())]),
            Some(first[0]),
        );
    }

    #[test]
    fn skips_obsolete_and_unusable_addresses() {
        let servers = [
            OBSOLETE_SERVERS[0],
            OBSOLETE_SERVERS[1],
            OBSOLETE_SERVERS[2],
            "::".parse().unwrap(),
            "fe80::53".parse().unwrap(),
            "127.0.0.1".parse().unwrap(),
        ];

        assert_eq!(
            first_resolver([(true, servers.as_slice())]),
            Some(servers[5])
        );
    }

    #[test]
    fn returns_none_without_a_usable_server() {
        assert_eq!(first_resolver([(true, &[][..])]), None);
        assert_eq!(first_resolver([(true, OBSOLETE_SERVERS.as_slice())]), None);
    }
}
