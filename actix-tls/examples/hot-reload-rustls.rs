//! Reload TLS credentials without restarting the server.
//!
//! See `examples/hot-reload.md` for run and verification instructions.

use std::{
    fs::File,
    io::{self, BufReader},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use actix_server::Server;
use actix_service::ServiceFactoryExt as _;
use actix_tls::accept::rustls_0_23::{Acceptor, TlsStream};
use arc_swap::ArcSwap;
use futures_util::future::ok;
use tokio::net::TcpStream;
use tokio_rustls_026::rustls::{
    self,
    crypto::CryptoProvider,
    server::{ClientHello, ResolvesServerCert},
    sign::CertifiedKey,
    ServerConfig,
};

#[tokio::main(flavor = "local")]
async fn main() -> io::Result<()> {
    let (cert, key) =
        parse_args().inspect_err(|_| eprintln!("usage: hot-reload-rustls CERT KEY"))?;
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let resolver = Arc::new(ReloadingResolver::new(&cert, &key, &provider)?);
    let config = ServerConfig::builder_with_provider(Arc::clone(&provider))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_cert_resolver(resolver.clone());
    let acceptor = Acceptor::new(config);

    let server = Server::build()
        .bind("tls-reload", ("127.0.0.1", 8443), move || {
            acceptor
                .clone()
                .map_err(|err| eprintln!("TLS error: {err:?}"))
                .and_then(|_stream: TlsStream<TcpStream>| ok(()))
        })?
        .workers(2)
        .run();

    let reload = spawn_reload(move || resolver.reload(&cert, &key, &provider));

    eprintln!("Listening on 127.0.0.1:8443; checking credentials every 2 seconds");
    let result = server.await;
    reload.abort();
    result
}

fn parse_args() -> io::Result<(PathBuf, PathBuf)> {
    let mut args = std::env::args_os().skip(1);
    let cert = args
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Invalid certificate path"))?;
    let key = args
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Invalid key path"))?;
    Ok((cert.into(), key.into()))
}

/// All workers use the same resolver. Each handshake loads an owned snapshot of the key.
#[derive(Debug)]
struct ReloadingResolver(ArcSwap<CertifiedKey>);

impl ResolvesServerCert for ReloadingResolver {
    fn resolve(&self, _hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        Some(self.0.load_full())
    }
}

impl ReloadingResolver {
    fn new(cert: &Path, key: &Path, provider: &CryptoProvider) -> io::Result<Self> {
        Ok(Self(ArcSwap::from_pointee(load_key(cert, key, provider)?)))
    }

    fn reload(&self, cert: &Path, key: &Path, provider: &CryptoProvider) -> io::Result<bool> {
        // Read and validate both files before publishing the replacement. A failed reload leaves
        // the current credentials in place, including when only one file has been replaced.
        let replacement = load_key(cert, key, provider)?;
        let current = self.0.load();
        if current.cert == replacement.cert {
            return Ok(false);
        }
        self.0.store(Arc::new(replacement));
        Ok(true)
    }
}

fn load_key(cert: &Path, key: &Path, provider: &CryptoProvider) -> io::Result<CertifiedKey> {
    let certs = rustls_pemfile::certs(&mut BufReader::new(File::open(cert)?))
        .collect::<io::Result<Vec<_>>>()?;
    let key = rustls_pemfile::private_key(&mut BufReader::new(File::open(key)?))?
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no private key found"))?;
    let certified = CertifiedKey::from_der(certs, key, provider)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    certified
        .keys_match()
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    Ok(certified)
}

/// Runs file access and key parsing outside the async runtime and TLS workers.
fn spawn_reload(
    reload: impl Fn() -> io::Result<bool> + Send + Sync + 'static,
) -> tokio::task::JoinHandle<()> {
    let reload = Arc::new(reload);
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let reload = Arc::clone(&reload);
            match tokio::task::spawn_blocking(move || reload()).await {
                Ok(Ok(true)) => eprintln!("TLS credentials reloaded"),
                Ok(Ok(false)) => {}
                Ok(Err(err)) => eprintln!("TLS reload failed; keeping current credentials: {err}"),
                Err(err) => eprintln!("TLS reload task failed: {err}"),
            }
        }
    })
}
