use std::io;

use actix_rt::net::TcpStream;
use actix_server::Server;
use actix_service::{fn_factory, fn_service};
use log::info;

#[actix_rt::main]
async fn main() -> io::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::new().default_filter_or("trace,mio=info"))
        .init();

    let addr = ("127.0.0.1", 8080);
    info!("starting server on port: {}", &addr.0);

    Server::build()
        .bind(
            "startup-fail",
            addr,
            fn_factory(|| async move {
                if 1 > 2 {
                    Ok(fn_service(move |mut _stream: TcpStream| async move {
                        Ok::<u32, u32>(0)
                    }))
                } else {
                    Err(42)
                }
            }),
        )?
        .workers(2)
        .run()
        .await
}
