//! Test configuration access on the host system.

#![cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]

#[test]
#[ignore = "requires a configured system DNS resolver"]
fn reads_system_configuration() {
    let _address = resolvconf::configured_dns_resolver();
}
