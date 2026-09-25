//! Simple composite-service TCP echo server.
//!
//! Run with `cargo run -p actix-server --example tcp-echo`. The default tracing filter shows
//! connection spans and their worker IDs. Set `RUST_LOG` to override the filter.
//!
//! Using the following command:
//!
//! ```sh
//! nc 127.0.0.1 8080
//! ```
//!
//! Start typing. When you press enter the typed line will be echoed back. The server will log
//! the length of each line it echos and the total size of data sent when the connection is closed.

use std::io;

use actix_rt::net::TcpStream;
use actix_server::Server;
use actix_service::{fn_service, ServiceFactoryExt as _};
use bytes::BytesMut;
use futures_util::future::ok;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tracing_subscriber::{fmt::format::FmtSpan, prelude::*, EnvFilter};

async fn run() -> io::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,actix_server=debug")),
        )
        .with_span_events(FmtSpan::NEW | FmtSpan::CLOSE)
        .with_ansi(false)
        .finish()
        .init();

    let addr = ("127.0.0.1", 8080);
    tracing::info!("starting server on: {}:{}", &addr.0, &addr.1);

    // Bind socket address and start worker(s). By default, the server uses the number of physical
    // CPU cores as the worker count. For this reason, the closure passed to bind needs to return
    // a service *factory*; so it can be created once per worker.
    Server::build()
        .bind("echo", addr, || {
            fn_service(move |mut stream: TcpStream| {
                async move {
                    let mut size = 0;
                    let mut buf = BytesMut::new();

                    loop {
                        match stream.read_buf(&mut buf).await {
                            // end of stream; bail from loop
                            Ok(0) => break,

                            // more bytes to process
                            Ok(bytes_read) => {
                                tracing::info!(bytes_read, "received data");
                                stream.write_all(&buf[size..]).await.unwrap();
                                size += bytes_read;
                            }

                            // stream error; bail from loop with error
                            Err(err) => {
                                tracing::error!("stream error: {:?}", err);
                                return Err(());
                            }
                        }
                    }

                    // send data down service pipeline
                    Ok((buf.freeze(), size))
                }
            })
            .map_err(|err| tracing::error!("service error: {:?}", err))
            .and_then(move |(_, size)| {
                tracing::info!(total_bytes = size, "connection finished");
                ok(size)
            })
        })?
        .workers(2)
        .run()
        .await
}

#[tokio::main]
async fn main() -> io::Result<()> {
    run().await?;
    Ok(())
}

// alternatively:
// #[actix_rt::main]
// async fn main() -> io::Result<()> {
//     run().await?;
//     Ok(())
// }
