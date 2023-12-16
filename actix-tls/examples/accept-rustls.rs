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

// this `use` is only exists because of how we have organised the crate
// it is not necessary for your actual code; you should import from `rustls` normally
use std::{
    fs::File,
    io::{self, BufReader},
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use actix_rt::net::TcpStream;
use actix_server::Server;
use actix_service::ServiceFactoryExt as _;
use actix_tls::accept::rustls_0_22::{Acceptor as RustlsAcceptor, TlsStream};
use futures_util::future::ok;
use itertools::Itertools as _;
use rustls::server::ServerConfig;
use rustls_pemfile::{certs, rsa_private_keys};
use rustls_pki_types_1::PrivateKeyDer;
use tokio_rustls_025::rustls;
use tracing::info;

#[actix_rt::main]
async fn main() -> io::Result<()> {
    env_logger::init_from_env(env_logger::Env::default().default_filter_or("info"));

    let root_path = env!("CARGO_MANIFEST_DIR")
        .parse::<PathBuf>()
        .unwrap()
        .join("examples");
    let cert_path = root_path.clone().join("cert.pem");
    let key_path = root_path.clone().join("key.pem");

    // Load TLS key and cert files
    let cert_file = &mut BufReader::new(File::open(cert_path).unwrap());
    let key_file = &mut BufReader::new(File::open(key_path).unwrap());

    let cert_chain = certs(cert_file);
    let mut keys = rsa_private_keys(key_file);

    let tls_config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            cert_chain.try_collect::<_, Vec<_>, _>()?,
            PrivateKeyDer::Pkcs1(keys.next().unwrap()?),
        )
        .unwrap();

    let tls_acceptor = RustlsAcceptor::new(tls_config);

    let count = Arc::new(AtomicUsize::new(0));

    let addr = ("127.0.0.1", 8443);
    info!("starting server at: {addr:?}");

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
