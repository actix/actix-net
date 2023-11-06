//! Use Rustls connector to test OpenSSL acceptor.

#![cfg(all(
    feature = "accept",
    feature = "connect",
    feature = "rustls-0_21",
    feature = "openssl"
))]

use std::{convert::TryFrom, io::Write, sync::Arc};

use actix_rt::net::TcpStream;
use actix_server::TestServer;
use actix_service::ServiceFactoryExt as _;
use actix_tls::accept::openssl::{Acceptor, TlsStream};
use actix_utils::future::ok;
use tokio_rustls::rustls::{Certificate, ClientConfig, RootCertStore, ServerName};
use tokio_rustls_024 as tokio_rustls;

fn new_cert_and_key() -> (String, String) {
    let cert =
        rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_owned(), "localhost".to_owned()])
            .unwrap();

    let key = cert.serialize_private_key_pem();
    let cert = cert.serialize_pem().unwrap();

    (cert, key)
}

fn openssl_acceptor(cert: String, key: String) -> tls_openssl::ssl::SslAcceptor {
    use tls_openssl::{
        pkey::PKey,
        ssl::{SslAcceptor, SslMethod},
        x509::X509,
    };

    let cert = X509::from_pem(cert.as_bytes()).unwrap();
    let key = PKey::private_key_from_pem(key.as_bytes()).unwrap();

    let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
    builder.set_certificate(&cert).unwrap();
    builder.set_private_key(&key).unwrap();
    builder.set_alpn_select_callback(|_, _protocols| Ok(b"http/1.1"));
    builder.set_alpn_protos(b"\x08http/1.1").unwrap();
    builder.build()
}

#[allow(dead_code)]
mod danger {
    use std::time::SystemTime;

    use tokio_rustls_024::rustls::{
        self,
        client::{ServerCertVerified, ServerCertVerifier},
    };

    use super::*;

    pub struct NoCertificateVerification;

    impl ServerCertVerifier for NoCertificateVerification {
        fn verify_server_cert(
            &self,
            _end_entity: &Certificate,
            _intermediates: &[Certificate],
            _server_name: &ServerName,
            _scts: &mut dyn Iterator<Item = &[u8]>,
            _ocsp_response: &[u8],
            _now: SystemTime,
        ) -> Result<ServerCertVerified, rustls::Error> {
            Ok(ServerCertVerified::assertion())
        }
    }
}

#[allow(dead_code)]
fn rustls_connector(_cert: String, _key: String) -> ClientConfig {
    let mut config = ClientConfig::builder()
        .with_safe_defaults()
        .with_root_certificates(RootCertStore::empty())
        .with_no_client_auth();

    config
        .dangerous()
        .set_certificate_verifier(Arc::new(danger::NoCertificateVerification));

    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    config
}

#[actix_rt::test]
async fn accepts_connections() {
    let (cert, key) = new_cert_and_key();

    let srv = TestServer::start({
        let cert = cert.clone();
        let key = key.clone();

        move || {
            let openssl_acceptor = openssl_acceptor(cert.clone(), key.clone());
            let tls_acceptor = Acceptor::new(openssl_acceptor);

            tls_acceptor
                .map_err(|err| println!("OpenSSL error: {:?}", err))
                .and_then(move |_stream: TlsStream<TcpStream>| ok(()))
        }
    });

    let mut sock = srv
        .connect()
        .expect("cannot connect to test server")
        .into_std()
        .unwrap();
    sock.set_nonblocking(false).unwrap();

    let config = rustls_connector(cert, key);
    let config = Arc::new(config);

    let mut conn = tokio_rustls::rustls::ClientConnection::new(
        config,
        ServerName::try_from("localhost").unwrap(),
    )
    .unwrap();

    let mut stream = tokio_rustls::rustls::Stream::new(&mut conn, &mut sock);

    stream.flush().expect("TLS handshake failed");
}
