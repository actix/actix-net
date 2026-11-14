//! Read the configured system DNS resolver address.
//!
//! The `configured_dns_resolver` function is available on Linux, macOS, and Windows.

#![cfg_attr(
    not(any(target_os = "linux", target_os = "macos", target_os = "windows", test)),
    no_std
)]

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
use std::net::IpAddr;

#[cfg(target_os = "linux")]
use self::linux as platform;
#[cfg(target_os = "macos")]
use self::macos as platform;
#[cfg(target_os = "windows")]
use self::windows as platform;

/// Returns the first usable DNS resolver address from the system configuration.
///
/// Reads the configuration on each call. This function is synchronous and does not
/// send DNS queries or test whether the server is reachable.
///
/// - Linux: reads the `nameserver` entries in `/etc/resolv.conf` in file order.
///   Local stub addresses are preserved.
/// - macOS: reads `ServerAddresses` from `State:/Network/Global/DNS` in the
///   SystemConfiguration dynamic store, in array order.
/// - Windows: reads adapters through `GetAdaptersAddresses` and selects the first
///   usable DNS address on an operational adapter, in the order returned by Windows.
///   The obsolete placeholders from `fec0:0:0:ffff::1` through
///   `fec0:0:0:ffff::3` are skipped.
///
/// Unspecified, multicast, IPv4 broadcast, and IPv6 link-local addresses are skipped.
/// IPv6 link-local addresses require an interface scope, which [`IpAddr`] cannot
/// represent. This single address does not describe domain-specific DNS routing,
/// search rules, ports, or encrypted DNS settings.
///
/// # Panics
///
/// Panics if the system configuration cannot be read or contains no usable DNS
/// resolver address.
///
/// # Examples
///
/// ```no_run
/// let resolver = resolvconf::configured_dns_resolver();
/// println!("{resolver}");
/// ```
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
#[track_caller]
pub fn configured_dns_resolver() -> IpAddr {
    platform::configured_dns_resolver()
        .unwrap_or_else(|err| panic!("cannot read the configured DNS resolver: {err}"))
}

/// Returns whether an address can identify a DNS server without an interface scope.
///
/// Rejects unspecified and multicast addresses, `255.255.255.255`, and IPv6 link-local
/// addresses. IPv6 link-local addresses require a scope that [`IpAddr`] cannot represent.
/// Loopback and private addresses are accepted. Reachability is not checked.
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn usable_address(address: IpAddr) -> bool {
    if address.is_unspecified() || address.is_multicast() {
        return false;
    }

    match address {
        IpAddr::V4(address) => !address.is_broadcast(),
        IpAddr::V6(address) => !address.is_unicast_link_local(),
    }
}
