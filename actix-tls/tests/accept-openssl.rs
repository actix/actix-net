//! Use Rustls connector to test OpenSSL acceptor.

#![cfg(all(
    feature = "accept",
    feature = "connect",
    feature = "rustls-0_22",
    feature = "openssl"
))]

use std::{io::Write as _, sync::Arc};

use actix_rt::net::TcpStream;
use actix_server::TestServer;
use actix_service::ServiceFactoryExt as _;
use actix_tls::{
    accept::openssl::{Acceptor, TlsStream},
    connect::rustls_0_22::reexports::ClientConfig,
};
use actix_utils::future::ok;
use rustls_pki_types_1::ServerName;
use tokio_rustls_025::rustls::RootCertStore;

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

mod danger {
    use rustls_pki_types_1::{CertificateDer, ServerName, UnixTime};
    use tokio_rustls_025::rustls;

    /// Disables certificate verification to allow self-signed certs from rcgen.
    #[derive(Debug)]
    pub struct NoCertificateVerification;

    impl rustls::client::danger::ServerCertVerifier for NoCertificateVerification {
        fn verify_server_cert(
            &self,
            _end_entity: &CertificateDer<'_>,
            _intermediates: &[CertificateDer<'_>],
            _server_name: &ServerName<'_>,
            _ocsp_response: &[u8],
            _now: UnixTime,
        ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            _message: &[u8],
            _cert: &rustls_pki_types_1::CertificateDer<'_>,
            _dss: &rustls::DigitallySignedStruct,
        ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
            Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
        }

        fn verify_tls13_signature(
            &self,
            _message: &[u8],
            _cert: &rustls_pki_types_1::CertificateDer<'_>,
            _dss: &rustls::DigitallySignedStruct,
        ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
            Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
        }

        fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
            rustls::crypto::ring::default_provider()
                .signature_verification_algorithms
                .supported_schemes()
        }
    }
}

#[allow(dead_code)]
fn rustls_connector(_cert: String, _key: String) -> ClientConfig {
    let mut config = ClientConfig::builder()
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

    let mut conn = tokio_rustls_025::rustls::ClientConnection::new(
        config,
        ServerName::try_from("localhost").unwrap(),
    )
    .unwrap();

    let mut stream = tokio_rustls_025::rustls::Stream::new(&mut conn, &mut sock);

    stream.flush().expect("TLS handshake failed");
}
