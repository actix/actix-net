//! Reload TLS credentials without restarting the server.
//!
//! See `examples/hot-reload.md` for run and verification instructions.

use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use actix_server::Server;
use actix_service::ServiceFactoryExt as _;
use actix_tls::accept::openssl::{Acceptor, TlsStream};
use arc_swap::ArcSwap;
use futures_util::future::ok;
use tls_openssl::ssl::{SniError, SslAcceptor, SslMethod};
use tokio::net::TcpStream;

#[tokio::main(flavor = "local")]
async fn main() -> io::Result<()> {
    let (cert, key) =
        parse_args().inspect_err(|_| eprintln!("usage: hot-reload-openssl CERT KEY"))?;
    let shared = Arc::new(ReloadingAcceptor::new(&cert, &key)?);
    let acceptor = Acceptor::new(shared.acceptor()?);

    let server = Server::build()
        .bind("tls-reload", ("127.0.0.1", 8443), move || {
            acceptor
                .clone()
                .map_err(|err| eprintln!("TLS error: {err:?}"))
                .and_then(|_stream: TlsStream<TcpStream>| ok(()))
        })?
        .workers(2)
        .run();

    let reload = spawn_reload(move || shared.reload(&cert, &key));

    eprintln!("Listening on 127.0.0.1:8443; checking credentials every 2 seconds");
    let result = server.await;
    reload.abort();
    result
}

/// Returns the parsed certificate and key paths from the command line.
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

/// All workers share the current context. Each handshake loads a snapshot.
struct ReloadingAcceptor {
    current: ArcSwap<Credentials>,
}

struct Credentials {
    acceptor: SslAcceptor,
    certificate_pem: Vec<u8>,
}

impl ReloadingAcceptor {
    fn new(cert: &Path, key: &Path) -> io::Result<Self> {
        Ok(Self {
            current: ArcSwap::from_pointee(load_acceptor(cert, key)?),
        })
    }

    fn acceptor(self: &Arc<Self>) -> io::Result<SslAcceptor> {
        let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls())?;
        let shared = Arc::clone(self);
        // Use the current context for every handshake, regardless of the requested server name.
        builder.set_servername_callback(move |ssl, _alert| {
            let current = shared.current.load();
            ssl.set_ssl_context(current.acceptor.context())
                .map_err(|_| SniError::ALERT_FATAL)
        });
        Ok(builder.build())
    }

    fn reload(&self, cert: &Path, key: &Path) -> io::Result<bool> {
        // Read and validate both files before replacing the shared context.
        let replacement = load_acceptor(cert, key)?;
        let current = self.current.load();
        if current.certificate_pem == replacement.certificate_pem {
            return Ok(false);
        }
        self.current.store(Arc::new(replacement));
        Ok(true)
    }
}

fn load_acceptor(cert: &Path, key: &Path) -> io::Result<Credentials> {
    let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls())?;
    let pem = fs::read(cert)?;
    let mut chain = tls_openssl::x509::X509::stack_from_pem(&pem)?.into_iter();
    let leaf = chain
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no certificate found"))?;
    builder.set_certificate(&leaf)?;
    for cert in chain {
        builder.add_extra_chain_cert(cert)?;
    }
    // Parse the key without an interactive password prompt.
    let key = tls_openssl::pkey::PKey::private_key_from_pem(&fs::read(key)?)?;
    builder.set_private_key(&key)?;
    builder.check_private_key()?;
    Ok(Credentials {
        acceptor: builder.build(),
        certificate_pem: pem,
    })
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
