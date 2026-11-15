//! Use a known provider's DoH endpoint when its DNS server address is configured.

use std::{error::Error, net::IpAddr};

use hickory_resolver::{
    config::{NameServerConfig, ProtocolConfig, ResolverConfig, CLOUDFLARE, GOOGLE, QUAD9},
    lookup_ip::LookupIp,
    net::runtime::TokioRuntimeProvider,
    TokioResolver,
};

#[tokio::main(flavor = "local")]
async fn main() -> Result<(), Box<dyn Error>> {
    let dns_server = resolvconf::system_dns_resolver()?;

    for address in lookup(dns_server).await?.iter() {
        println!("{address}");
    }

    Ok(())
}

/// Looks up example.com through the supplied DNS server, using DoH for known providers.
async fn lookup(dns_server: IpAddr) -> Result<LookupIp, Box<dyn Error>> {
    println!("DNS resolver: {dns_server}");

    let name_server = name_server_config(dns_server);

    match &name_server.connections[0].protocol {
        ProtocolConfig::Https { server_name, path } => {
            println!("DoH endpoint: https://{server_name}{path}");
        }
        _ => println!("DNS transport: UDP/TCP"),
    }

    let config = ResolverConfig::from_name_servers(vec![name_server]);
    let resolver =
        TokioResolver::builder_with_config(config, TokioRuntimeProvider::default()).build()?;

    Ok(resolver.lookup_ip("example.com").await?)
}

/// Selects a known provider's DoH endpoint or keeps the supplied server on UDP/TCP.
fn name_server_config(dns_server: IpAddr) -> NameServerConfig {
    let provider = [CLOUDFLARE, GOOGLE, QUAD9]
        .into_iter()
        .find(|provider| provider.ips.contains(&dns_server));

    match provider {
        Some(provider) => NameServerConfig::https(
            dns_server,
            provider.server_name.into(),
            Some(provider.path.into()),
        ),
        None => NameServerConfig::udp_and_tcp(dns_server),
    }
}
