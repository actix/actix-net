//! Adds PROXY protocol v1 prelude to connections.

#![allow(unused)]

use std::{
    io, mem,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use actix_proxy_protocol::{tlv, v1, v2, AddressFamily, Command, TransportProtocol};
use actix_rt::net::TcpStream;
use actix_server::Server;
use actix_service::{fn_service, ServiceFactoryExt as _};
use bytes::BytesMut;
use const_str::concat_bytes;
use once_cell::sync::Lazy;
use tokio::io::{copy_bidirectional, AsyncReadExt as _, AsyncWriteExt as _};

static UPSTREAM: Lazy<SocketAddr> = Lazy::new(|| SocketAddr::from(([127, 0, 0, 1], 8080)));

/*
NOTES:
108 byte buffer on receiver side is enough for any PROXY header
after PROXY, receive until CRLF, *then* decode parts
TLV = type-length-value

TO DO:
handle UNKNOWN transport
v2 UNSPEC mode
AF_UNIX socket
*/

fn extend_with_ip_bytes(buf: &mut Vec<u8>, ip: IpAddr) {
    match ip {
        IpAddr::V4(ip) => buf.extend_from_slice(&ip.octets()),
        IpAddr::V6(ip) => buf.extend_from_slice(&ip.octets()),
    }
}

async fn wrap_with_proxy_protocol_v1(mut stream: TcpStream) -> io::Result<()> {
    let mut upstream = TcpStream::connect(("127.0.0.1", 8080)).await?;

    tracing::info!(
        "PROXYv1 {} -> {}",
        stream.peer_addr().unwrap(),
        UPSTREAM.to_string()
    );

    let proxy_header = v1::Header::new(
        AddressFamily::Inet,
        SocketAddr::from(([127, 0, 0, 1], 8081)),
        *UPSTREAM,
    );

    proxy_header.write_to_tokio(&mut upstream).await?;

    let (_bytes_read, _bytes_written) = copy_bidirectional(&mut stream, &mut upstream).await?;

    Ok(())
}

async fn wrap_with_proxy_protocol_v2(mut stream: TcpStream) -> io::Result<()> {
    let mut upstream = TcpStream::connect(("127.0.0.1", 8080)).await?;

    tracing::info!(
        "PROXYv2 {} -> {}",
        stream.peer_addr().unwrap(),
        UPSTREAM.to_string()
    );

    let mut proxy_header = v2::Header::new_tcp_ipv4_proxy(([127, 0, 0, 1], 8082), *UPSTREAM);

    proxy_header.add_tlv(0x05, [0x34, 0x32, 0x36, 0x39]); // UNIQUE_ID
    proxy_header.add_tlv(0x04, "NOOP m9"); // NOOP
    proxy_header.add_crc23c_checksum_tlv();

    proxy_header.write_to_tokio(&mut upstream).await?;

    let (_bytes_read, _bytes_written) = copy_bidirectional(&mut stream, &mut upstream).await?;

    Ok(())
}

fn start_server() -> io::Result<Server> {
    let addr = ("127.0.0.1", 8082);
    tracing::info!("starting proxy server on port: {}", &addr.0);
    tracing::info!("proxying to 127.0.0.1:8080");

    Ok(Server::build()
        .bind("proxy-protocol-v1", ("127.0.0.1", 8081), move || {
            fn_service(wrap_with_proxy_protocol_v1)
                .map_err(|err| tracing::error!("service error: {:?}", err))
        })?
        .bind("proxy-protocol-v2", addr, move || {
            fn_service(wrap_with_proxy_protocol_v2)
                .map_err(|err| tracing::error!("service error: {:?}", err))
        })?
        .workers(2)
        .run())
}

#[tokio::main]
async fn main() -> io::Result<()> {
    env_logger::init_from_env(env_logger::Env::default().default_filter_or("info"));
    start_server()?.await?;
    Ok(())
}
