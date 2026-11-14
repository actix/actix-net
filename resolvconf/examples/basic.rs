//! Print the configured system DNS resolver address.

fn main() {
    let resolver = resolvconf::configured_dns_resolver();

    println!("{resolver}");
}
