//! Tests for Rustls acceptor.

#![cfg(all(feature = "accept", feature = "rustls-0_23",))]

#[cfg(feature = "openssl")]
extern crate tls_openssl as openssl;

use core::future::ready;
use std::{io::BufReader, sync::mpsc, time::Duration};

#[cfg(all(feature = "connect", feature = "openssl"))]
use std::io::Write;

use actix_rt::net::TcpStream;
use actix_server::TestServer;
use actix_service::ServiceFactoryExt as _;
use actix_tls::accept::{
    rustls_0_23::{reexports::ServerConfig, Acceptor, TlsStream},
    TlsError,
};
use rustls_pemfile::{certs, pkcs8_private_keys};
use rustls_pki_types_1::PrivateKeyDer;

fn new_cert_and_key() -> (String, String) {
    let rcgen::CertifiedKey { cert, signing_key } =
        rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_owned(), "localhost".to_owned()])
            .unwrap();

    let key = signing_key.serialize_pem();
    let cert = cert.pem();

    (cert, key)
}

fn rustls_server_config(cert: String, key: String) -> ServerConfig {
    // Load TLS key and cert files

    let cert = &mut BufReader::new(cert.as_bytes());
    let key = &mut BufReader::new(key.as_bytes());

    let cert_chain = certs(cert).collect::<Result<Vec<_>, _>>().unwrap();
    let mut keys = pkcs8_private_keys(key)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    let mut config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(cert_chain, PrivateKeyDer::Pkcs8(keys.remove(0)))
        .unwrap();

    config.alpn_protocols = vec![b"http/1.1".to_vec()];

    config
}

fn init_crypto() {
    let _ = tokio_rustls_026::rustls::crypto::aws_lc_rs::default_provider().install_default();
}

#[cfg(all(feature = "connect", feature = "openssl"))]
fn openssl_connector(
    cert: String,
    key: String,
) -> actix_tls::connect::openssl::reexports::SslConnector {
    use actix_tls::connect::openssl::reexports::{SslConnector, SslMethod};
    use openssl::{pkey::PKey, ssl::SslVerifyMode, x509::X509};

    let cert = X509::from_pem(cert.as_bytes()).unwrap();
    let key = PKey::private_key_from_pem(key.as_bytes()).unwrap();

    let mut ssl = SslConnector::builder(SslMethod::tls()).unwrap();
    ssl.set_verify(SslVerifyMode::NONE);
    ssl.set_certificate(&cert).unwrap();
    ssl.set_private_key(&key).unwrap();
    ssl.set_alpn_protos(b"\x08http/1.1").unwrap();

    ssl.build()
}

#[cfg(all(feature = "connect", feature = "openssl"))]
#[actix_rt::test]
async fn accepts_connections() {
    init_crypto();

    let (cert, key) = new_cert_and_key();

    let srv = TestServer::start({
        let cert = cert.clone();
        let key = key.clone();

        move || {
            let tls_acceptor = Acceptor::new(rustls_server_config(cert.clone(), key.clone()));

            tls_acceptor
                .map_err(|err| println!("Rustls error: {err:?}"))
                .and_then(move |_stream: TlsStream<TcpStream>| ready(Ok(())))
        }
    });

    let sock = srv
        .connect()
        .expect("cannot connect to test server")
        .into_std()
        .unwrap();
    sock.set_nonblocking(false).unwrap();

    let connector = openssl_connector(cert, key);

    let mut stream = connector
        .connect("localhost", sock)
        .expect("TLS handshake failed");

    stream.do_handshake().expect("TLS handshake failed");

    stream.flush().expect("TLS handshake failed");
}

#[actix_rt::test]
async fn handshake_timeout() {
    init_crypto();
    let (cert, key) = new_cert_and_key();
    let (tx, rx) = mpsc::channel();

    let srv = TestServer::start({
        let cert = cert.clone();
        let key = key.clone();

        move || {
            let tx = tx.clone();
            let mut tls_acceptor = Acceptor::new(rustls_server_config(cert.clone(), key.clone()));
            tls_acceptor.set_handshake_timeout(Duration::from_millis(50));

            tls_acceptor
                .map_err(move |err| {
                    let _ = tx.send(err);
                })
                .and_then(move |_stream: TlsStream<TcpStream>| ready(Ok(())))
        }
    });

    let _sock = srv
        .connect()
        .expect("cannot connect to test server")
        .into_std()
        .unwrap();

    let err = rx
        .recv_timeout(Duration::from_secs(2))
        .expect("server should emit timeout error for stalled handshake");

    assert!(matches!(err, TlsError::Timeout));
}
