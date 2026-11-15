# resolvconf

<!-- prettier-ignore-start -->

[![crates.io](https://img.shields.io/crates/v/resolvconf?label=latest)](https://crates.io/crates/resolvconf)
[![Documentation](https://docs.rs/resolvconf/badge.svg?version=0.1.1)](https://docs.rs/resolvconf/0.1.1)
![Version](https://img.shields.io/badge/rustc-1.89+-ab6000.svg)
![MIT or Apache 2.0 licensed](https://img.shields.io/crates/l/resolvconf.svg)
<br />
[![Dependency Status](https://deps.rs/crate/resolvconf/0.1.1/status.svg)](https://deps.rs/crate/resolvconf/0.1.1)
![Download](https://img.shields.io/crates/d/resolvconf.svg)
[![Chat on Discord](https://img.shields.io/discord/771444961383153695?label=chat&logo=discord)](https://discord.gg/NWpN5mmg3x)

<!-- prettier-ignore-end -->

<!-- cargo-rdme start -->

Read the system's configured DNS resolver address.

The [`system_dns_resolver`] function is available on Linux, macOS, and Windows.

## Examples

```rust
let resolver = resolvconf::system_dns_resolver()?;
println!("DNS resolver = {resolver}"); // E.g.: DNS resolver = 10.0.1.53
```

<!-- cargo-rdme end -->
