//! Print the configured system DNS resolver address.

fn main() {
    let resolver = resolvconf::system_dns_resolver();

    println!("{resolver}");
}
