//! Test configuration access on the host system.

#![cfg(any(
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "linux",
    target_os = "macos",
    target_os = "netbsd",
    target_os = "openbsd",
    target_os = "windows",
))]

#[test]
fn reads_system_configuration() -> std::io::Result<()> {
    let _address = resolvconf::system_dns_resolver()?;

    Ok(())
}
