//! Demonstrates use of the `ServerBuilder::shutdown_signal` method using `tokio-util`s
//! `CancellationToken` helper using a nonsensical timer. In practice, this cancellation token would
//! be wired throughout your application and typically triggered by OS signals elsewhere.

use std::{io, time::Duration};

use actix_rt::net::TcpStream;
use actix_server::Server;
use actix_service::fn_service;
use tokio_util::sync::CancellationToken;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::{prelude::*, EnvFilter};

async fn run(stop_signal: CancellationToken) -> io::Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(
            EnvFilter::builder()
                .with_default_directive(LevelFilter::INFO.into())
                .from_env_lossy(),
        )
        .init();

    let addr = ("127.0.0.1", 8080);
    tracing::info!("starting server on port: {}", &addr.0);

    Server::build()
        .bind("shutdown-signal", addr, || {
            fn_service(|_stream: TcpStream| async { Ok::<_, io::Error>(()) })
        })?
        .shutdown_signal(stop_signal.cancelled_owned())
        .workers(2)
        .run()
        .await
}

#[tokio::main]
async fn main() -> io::Result<()> {
    let stop_signal = CancellationToken::new();

    tokio::spawn({
        let stop_signal = stop_signal.clone();
        async move {
            tokio::time::sleep(Duration::from_secs(10)).await;
            stop_signal.cancel();
        }
    });

    run(stop_signal).await?;
    Ok(())
}
