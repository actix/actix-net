//! Read the system's configured DNS resolver address.
//!
//! The [`system_dns_resolver`] function is available on Linux, macOS, and Windows.
//!
//! # Examples
//!
//! ```no_run
//! # #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
//! # {
//! let resolver = resolvconf::system_dns_resolver()?;
//! println!("DNS resolver = {resolver}"); // E.g.: DNS resolver = 10.0.1.53
//! # }
//! # Ok::<(), std::io::Error>(())
//! ```

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
use std::{io, net::IpAddr};

#[cfg(target_os = "linux")]
use self::linux as platform;
#[cfg(target_os = "macos")]
use self::macos as platform;
#[cfg(target_os = "windows")]
use self::windows as platform;

/// Returns the first usable DNS resolver address from the system configuration.
///
/// A fresh resolver config is obtained on each call.
///
/// Does not send DNS queries or test whether the server is reachable.
///
/// Due to file reads and syscalls performed, it is advisable to run this in a `spawn_blocking` (or
/// equivalent) if run in an async context.
///
/// Unspecified, multicast, IPv4 broadcast, and IPv6 link-local addresses are skipped (IPv6
/// link-local addresses require an interface scope, which [`IpAddr`] cannot represent).
///
/// # Platform Behavior
///
/// - Linux: reads `nameserver` entries from `/etc/resolv.conf`, including local stub addresses.
/// - macOS: reads `ServerAddresses` from `State:/Network/Global/DNS` in the SystemConfiguration
///   dynamic store.
/// - Windows: reads adapters through `GetAdaptersAddresses` and selects the first usable DNS
///   address on an operational adapter. The obsolete placeholders from `fec0:0:0:ffff::1` through
///   `fec0:0:0:ffff::3` are also filtered out.
///
/// # Errors
///
/// Returns an error if the system configuration cannot be read or contains no usable DNS resolver
/// address.
///
/// # Examples
///
/// ```no_run
/// let resolver = resolvconf::system_dns_resolver()?;
/// println!("{resolver}");
/// # Ok::<(), std::io::Error>(())
/// ```
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
pub fn system_dns_resolver() -> io::Result<IpAddr> {
    platform::system_dns_resolver()
}

/// Returns whether an address can identify a DNS server without an interface scope.
///
/// Rejects unspecified and multicast addresses, `255.255.255.255`, and IPv6 link-local addresses.
/// IPv6 link-local addresses require a scope that [`IpAddr`] cannot represent. Loopback and private
/// addresses are accepted. Reachability is not checked.
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
