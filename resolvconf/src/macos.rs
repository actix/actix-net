use std::{io, net::IpAddr};

use system_configuration::{
    core_foundation::{
        array::CFArray,
        base::{CFType, TCFType},
        dictionary::CFDictionary,
        propertylist::CFPropertyList,
        string::CFString,
    },
    dynamic_store::SCDynamicStoreBuilder,
};

pub(super) fn system_dns_resolver() -> io::Result<IpAddr> {
    let store = SCDynamicStoreBuilder::new("resolvconf")
        .build()
        .ok_or_else(|| io::Error::other("Cannot open the SystemConfiguration dynamic store"))?;

    let config = store
        .get("State:/Network/Global/DNS")
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "No global DNS configuration"))?;

    from_dns_config(config)
}

fn from_dns_config(config: CFPropertyList) -> io::Result<IpAddr> {
    let config = config.downcast_into::<CFDictionary>().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "DNS configuration is not a dictionary",
        )
    })?;

    // SAFETY: A property-list dictionary has CFString keys and CFType values.
    // The new wrapper retains the dictionary before the original wrapper is dropped.
    let config = unsafe {
        CFDictionary::<CFString, CFType>::wrap_under_get_rule(config.as_concrete_TypeRef())
    };

    let servers = config
        .find(CFString::from_static_string("ServerAddresses"))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "No DNS ServerAddresses"))?
        .downcast::<CFArray>()
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "DNS ServerAddresses is not an array",
            )
        })?;

    // SAFETY: Each element of a property-list array is a CFType. Each element's
    // concrete type is checked before it is read as a string.
    let servers = unsafe { CFArray::<CFType>::wrap_under_get_rule(servers.as_concrete_TypeRef()) };

    servers
        .iter()
        .filter_map(|server| server.downcast::<CFString>())
        .filter_map(|server| server.to_string().parse().ok())
        .find(|&ip| crate::usable_address(ip))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "No usable global DNS server"))
}

#[cfg(test)]
mod tests {
    use system_configuration::core_foundation::{
        boolean::CFBoolean, propertylist::CFPropertyListSubClass,
    };

    use super::*;

    fn dns_config(servers: CFType) -> CFPropertyList {
        CFDictionary::from_CFType_pairs(&[(
            CFString::from_static_string("ServerAddresses"),
            servers,
        )])
        .to_untyped()
        .into_CFPropertyList()
    }

    #[test]
    fn preserves_global_server_order() {
        let servers =
            CFArray::from_CFTypes(&[CFString::new("2001:db8::53"), CFString::new("192.0.2.53")]);

        assert_eq!(
            from_dns_config(dns_config(servers.as_CFType())).unwrap(),
            "2001:db8::53".parse::<IpAddr>().unwrap()
        );
    }

    #[test]
    fn skips_invalid_values_and_unusable_addresses() {
        let servers = CFArray::from_CFTypes(&[
            CFBoolean::true_value().as_CFType(),
            CFString::new("invalid").as_CFType(),
            CFString::new("fe80::53%en0").as_CFType(),
            CFString::new("0.0.0.0").as_CFType(),
            CFString::new("127.0.0.1").as_CFType(),
        ]);

        assert_eq!(
            from_dns_config(dns_config(servers.as_CFType())).unwrap(),
            "127.0.0.1".parse::<IpAddr>().unwrap()
        );
    }

    #[test]
    fn rejects_missing_empty_and_wrongly_typed_configuration() {
        let missing = CFDictionary::<CFString, CFType>::from_CFType_pairs(&[])
            .to_untyped()
            .into_CFPropertyList();
        let empty = CFArray::<CFType>::from_CFTypes(&[]);

        for config in [missing, dns_config(empty.as_CFType())] {
            assert_eq!(
                from_dns_config(config).unwrap_err().kind(),
                io::ErrorKind::NotFound
            );
        }

        for config in [
            CFString::new("invalid").into_CFPropertyList(),
            dns_config(CFString::new("192.0.2.53").as_CFType()),
        ] {
            assert_eq!(
                from_dns_config(config).unwrap_err().kind(),
                io::ErrorKind::InvalidData
            );
        }
    }
}
