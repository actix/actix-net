//! Use OpenSSL connector to test Rustls acceptor.

#![cfg(all(
    feature = "accept",
    feature = "connect",
    feature = "rustls-0_22",
    feature = "openssl"
))]

extern crate tls_openssl as openssl;

use std::io::{BufReader, Write};

use actix_rt::net::TcpStream;
use actix_server::TestServer;
use actix_service::ServiceFactoryExt as _;
use actix_tls::{
    accept::rustls_0_22::{reexports::ServerConfig, Acceptor, TlsStream},
    connect::openssl::reexports::SslConnector,
};
use actix_utils::future::ok;
use rustls_pemfile::{certs, pkcs8_private_keys};
use rustls_pki_types_1::PrivateKeyDer;
use tls_openssl::ssl::SslVerifyMode;

fn new_cert_and_key() -> (String, String) {
    let cert =
        rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_owned(), "localhost".to_owned()])
            .unwrap();

    let key = cert.serialize_private_key_pem();
    let cert = cert.serialize_pem().unwrap();

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

fn openssl_connector(cert: String, key: String) -> SslConnector {
    use actix_tls::connect::openssl::reexports::SslMethod;
    use openssl::{pkey::PKey, x509::X509};

    let cert = X509::from_pem(cert.as_bytes()).unwrap();
    let key = PKey::private_key_from_pem(key.as_bytes()).unwrap();

    let mut ssl = SslConnector::builder(SslMethod::tls()).unwrap();
    ssl.set_verify(SslVerifyMode::NONE);
    ssl.set_certificate(&cert).unwrap();
    ssl.set_private_key(&key).unwrap();
    ssl.set_alpn_protos(b"\x08http/1.1").unwrap();

    ssl.build()
}

#[actix_rt::test]
async fn accepts_connections() {
    let (cert, key) = new_cert_and_key();

    let srv = TestServer::start({
        let cert = cert.clone();
        let key = key.clone();

        move || {
            let tls_acceptor = Acceptor::new(rustls_server_config(cert.clone(), key.clone()));

            tls_acceptor
                .map_err(|err| println!("Rustls error: {:?}", err))
                .and_then(move |_stream: TlsStream<TcpStream>| ok(()))
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
