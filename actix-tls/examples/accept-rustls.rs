//! No-Op TLS Acceptor Server
//!
//! Using either HTTPie (`http`) or cURL:
//!
//! This commands will produce errors in the server log:
//! ```sh
//! curl 127.0.0.1:8443
//! http 127.0.0.1:8443
//! ```
//!
//! These commands will show "empty reply" on the client but will debug print the TLS stream info
//! in the server log, indicating a successful TLS handshake:
//! ```sh
//! curl -k https://127.0.0.1:8443
//! http --verify=false https://127.0.0.1:8443
//! ```

// this use only exists because of how we have organised the crate
// it is not necessary for your actual code
use tokio_rustls::rustls;

use std::{
    env,
    fs::File,
    io::{self, BufReader},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use actix_rt::net::TcpStream;
use actix_server::Server;
use actix_service::ServiceFactoryExt as _;
use actix_tls::accept::rustls::{Acceptor as RustlsAcceptor, TlsStream};
use futures_util::future::ok;
use log::info;
use rustls::{server::ServerConfig, Certificate, PrivateKey};
use rustls_pemfile::{certs, rsa_private_keys};

#[actix_rt::main]
async fn main() -> io::Result<()> {
    env::set_var("RUST_LOG", "info");
    env_logger::init();

    // Load TLS key and cert files
    let cert_file = &mut BufReader::new(File::open("./examples/cert.pem").unwrap());
    let key_file = &mut BufReader::new(File::open("./examples/key.pem").unwrap());

    let cert_chain = certs(cert_file)
        .unwrap()
        .into_iter()
        .map(Certificate)
        .collect();
    let mut keys = rsa_private_keys(key_file).unwrap();

    let tls_config = ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_single_cert(cert_chain, PrivateKey(keys.remove(0)))
        .unwrap();

    let tls_acceptor = RustlsAcceptor::new(tls_config);

    let count = Arc::new(AtomicUsize::new(0));

    let addr = ("127.0.0.1", 8443);
    info!("starting server on port: {}", &addr.0);

    Server::build()
        .bind("tls-example", addr, move || {
            let count = Arc::clone(&count);

            // Set up TLS service factory
            tls_acceptor
                .clone()
                .map_err(|err| println!("Rustls error: {:?}", err))
                .and_then(move |stream: TlsStream<TcpStream>| {
                    let num = count.fetch_add(1, Ordering::Relaxed);
                    info!("[{}] Got TLS connection: {:?}", num, &*stream);
                    ok(())
                })
        })?
        .workers(1)
        .run()
        .await
}
