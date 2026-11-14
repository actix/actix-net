//! Test configuration access on the host system.

#![cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]

#[test]
fn reads_system_configuration() {
    let _address = resolvconf::system_dns_resolver();
}
