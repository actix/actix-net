//! Simple composite-service TCP echo server.
//!
//! Using the following command:
//!
//! ```sh
//! nc 127.0.0.1 8080
//! ```
//!
//! Start typing. When you press enter the typed line will be echoed back. The server will log
//! the length of each line it echos and the total size of data sent when the connection is closed.

use std::{
    io,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use actix_rt::net::TcpStream;
use actix_server::Server;
use actix_service::{fn_service, ServiceFactoryExt as _};
use bytes::BytesMut;
use futures_util::future::ok;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

async fn run() -> io::Result<()> {
    pretty_env_logger::formatted_timed_builder()
        .parse_env(pretty_env_logger::env_logger::Env::default().default_filter_or("info"));

    let count = Arc::new(AtomicUsize::new(0));

    let addr = ("127.0.0.1", 8080);
    tracing::info!("starting server on port: {}", &addr.0);

    // Bind socket address and start worker(s). By default, the server uses the number of physical
    // CPU cores as the worker count. For this reason, the closure passed to bind needs to return
    // a service *factory*; so it can be created once per worker.
    Server::build()
        .bind("echo", addr, move || {
            let count = Arc::clone(&count);
            let num2 = Arc::clone(&count);

            fn_service(move |mut stream: TcpStream| {
                let count = Arc::clone(&count);

                async move {
                    let num = count.fetch_add(1, Ordering::SeqCst);
                    let num = num + 1;

                    let mut size = 0;
                    let mut buf = BytesMut::new();

                    loop {
                        match stream.read_buf(&mut buf).await {
                            // end of stream; bail from loop
                            Ok(0) => break,

                            // more bytes to process
                            Ok(bytes_read) => {
                                tracing::info!("[{}] read {} bytes", num, bytes_read);
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
                let num = num2.load(Ordering::SeqCst);
                tracing::info!("[{}] total bytes read: {}", num, size);
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
