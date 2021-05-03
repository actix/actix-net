//! A Tcp Server using mio::net::TcpListener.
//!
//! actix-server is used to bridge `mio` and multiple tokio current-thread runtime
//! for a thread per core like server.
//!
//! Server would return "Hello World!" String.

use std::{env, io};

use actix_rt::net::TcpStream;
use actix_server::ServerBuilder;
use actix_service::fn_service;
use tokio::io::AsyncWriteExt;

use mio::net::TcpListener;

// A dummy buffer always return hello world to client.
const BUF: &[u8] = b"HTTP/1.1 200 OK\r\n\
    content-length: 12\r\n\
    connection: close\r\n\
    date: Thu, 01 Jan 1970 12:34:56 UTC\r\n\
    \r\n\
    Hello World!";

#[actix_rt::main]
async fn main() -> io::Result<()> {
    env::set_var("RUST_LOG", "info");
    env_logger::init();

    let name = "hello_world";

    let addr = "127.0.0.1:8080".parse().unwrap();

    let lst = TcpListener::bind(addr)?;

    ServerBuilder::new()
        .bind_acceptable(name, addr, lst, || fn_service(response))
        .run()
        .await
}

async fn response(mut stream: TcpStream) -> io::Result<()> {
    stream.write(BUF).await?;
    stream.flush().await?;
    stream.shutdown().await
}
