//! Simple file-reader TCP server with framed stream.
//!
//! Using the following command:
//!
//! ```sh
//! nc 127.0.0.1 8080
//! ```
//!
//! Follow the prompt and enter a file path, relative or absolute.

use std::io;

use actix_codec::{Framed, LinesCodec};
use actix_rt::net::TcpStream;
use actix_server::Server;
use actix_service::{fn_service, ServiceFactoryExt as _};
use futures_util::{SinkExt as _, StreamExt as _};
use tokio::{fs::File, io::AsyncReadExt as _};

async fn run() -> io::Result<()> {
    pretty_env_logger::formatted_timed_builder()
        .parse_env(pretty_env_logger::env_logger::Env::default().default_filter_or("info"));

    let addr = ("127.0.0.1", 8080);
    tracing::info!("starting server on port: {}", &addr.0);

    // Bind socket address and start worker(s). By default, the server uses the number of physical
    // CPU cores as the worker count. For this reason, the closure passed to bind needs to return
    // a service *factory*; so it can be created once per worker.
    Server::build()
        .bind("file-reader", addr, move || {
            fn_service(move |stream: TcpStream| async move {
                // set up codec to use with I/O resource
                let mut framed = Framed::new(stream, LinesCodec::default());

                loop {
                    // prompt for file name
                    framed.send("Type file name to return:").await?;

                    // wait for next line
                    match framed.next().await {
                        Some(Ok(line)) => {
                            match File::open(&line).await {
                                Ok(mut file) => {
                                    tracing::info!("reading file: {}", &line);

                                    // read file into String buffer
                                    let mut buf = String::new();
                                    file.read_to_string(&mut buf).await?;

                                    // send String into framed object
                                    framed.send(buf).await?;

                                    // break out of loop and
                                    break;
                                }
                                Err(err) => {
                                    tracing::error!("{}", err);
                                    framed
                                        .send("File not found or not readable. Try again.")
                                        .await?;
                                    continue;
                                }
                            };
                        }

                        // not being able to read a line from the stream is unrecoverable
                        Some(Err(err)) => return Err(err),

                        // This EOF won't be hit.
                        None => continue,
                    }
                }

                // close connection after file has been copied to TCP stream
                Ok(())
            })
            .map_err(|err| tracing::error!("service error: {:?}", err))
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
