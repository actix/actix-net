//! Print the configured system DNS resolver address.

fn main() -> std::io::Result<()> {
    let resolver = resolvconf::system_dns_resolver()?;

    println!("{resolver}");

    Ok(())
}
