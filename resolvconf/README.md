# resolvconf

Read one DNS resolver address from the system configuration on Linux, macOS, and Windows.

```rust
let resolver = resolvconf::configured_dns_resolver();
```

Run the example to print the configured resolver address:

```console
cargo run -p resolvconf --example basic
```

Use that address with Hickory to look up `example.com`:

```console
cargo run -p resolvconf --example hickory
```

The DoH example checks whether the configured IP is a standard
[Cloudflare](https://developers.cloudflare.com/1.1.1.1/ip-addresses/),
[Google](https://developers.google.com/speed/public-dns/docs/using), or
[Quad9](https://docs.quad9.net/services/) DNS server.
A match uses that IP with the provider's HTTPS endpoint and certificate hostname.
Other addresses use UDP/TCP. Local stubs and routers do not identify their upstream provider.
DoH errors are returned without a retry over UDP/TCP.

```console
cargo run -p resolvconf --example hickory-doh
```

The function returns `std::net::IpAddr`. It reads the configuration on each call:

- Linux: the first usable `nameserver` in `/etc/resolv.conf`. Local stubs are preserved.
- macOS: the first usable `ServerAddresses` entry in the global SystemConfiguration DNS state.
- Windows: the first usable DNS server on an operational adapter, in the order returned by Windows.

On other platforms, the crate builds without providing `configured_dns_resolver`.

The function is synchronous. It panics if configuration access fails or no usable address exists.
It does not send DNS queries or test server reachability.

Unspecified, multicast, IPv4 broadcast, and IPv6 link-local addresses are skipped.
IPv6 link-local addresses need an interface scope that `IpAddr` cannot represent.
Windows' obsolete site-local DNS placeholders are also skipped.

A single IP address cannot describe domain-specific DNS routing, search rules, ports, or encryption settings.

## Platform references

- [Linux resolver configuration](https://man7.org/linux/man-pages/man5/resolv.conf.5.html)
- [Apple DNS schema](https://github.com/apple-oss-distributions/configd/blob/main/SystemConfiguration.fproj/SCSchemaDefinitions.h)
- [Windows adapter enumeration](https://learn.microsoft.com/en-us/windows/win32/api/iphlpapi/nf-iphlpapi-getadaptersaddresses)
