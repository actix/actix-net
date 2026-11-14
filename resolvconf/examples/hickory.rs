//! Look up example.com with the configured system DNS resolver.

use std::error::Error;

use hickory_resolver::{
    config::{NameServerConfig, ResolverConfig},
    net::runtime::TokioRuntimeProvider,
    TokioResolver,
};

#[tokio::main(flavor = "local")]
async fn main() -> Result<(), Box<dyn Error>> {
    let dns_server = resolvconf::configured_dns_resolver();

    println!("DNS resolver: {dns_server}");

    let config = ResolverConfig::from_name_servers(vec![NameServerConfig::udp_and_tcp(dns_server)]);
    let resolver =
        TokioResolver::builder_with_config(config, TokioRuntimeProvider::default()).build()?;

    for address in resolver.lookup_ip("example.com").await?.iter() {
        println!("{address}");
    }

    Ok(())
}
