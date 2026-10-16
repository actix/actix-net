//! Tests for the per-thread TLS handshake limit.

#![cfg(all(feature = "accept", feature = "rustls-0_23"))]

use std::{net, sync::mpsc, task::Context, thread, time::Duration};

use actix_rt::net::TcpStream;
use actix_service::{Service, ServiceFactory};
use actix_tls::accept::{
    max_concurrent_tls_connect,
    rustls_0_23::{reexports::ServerConfig, Acceptor, AcceptorService},
};
use futures_util::poll;
use tokio::runtime::LocalRuntime;
use tokio_test::{assert_pending, assert_ready_ok};

// Keep this as the only test in this executable. The setting is process-wide.
#[test]
fn initialized_threads_keep_their_tls_handshake_limit() {
    tokio_rustls_026::rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .unwrap();

    let rcgen::CertifiedKey { cert, signing_key } =
        rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap();
    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![cert.der().clone()],
            signing_key.serialize_der().try_into().unwrap(),
        )
        .unwrap();

    max_concurrent_tls_connect(1);

    let (initialized_tx, initialized_rx) = mpsc::channel();
    let (updated_tx, updated_rx) = mpsc::channel();
    let first_config = config.clone();

    let initialized_thread = thread::spawn(move || {
        let runtime = LocalRuntime::new().unwrap();

        runtime.block_on(async move {
            let acceptor = Acceptor::new(first_config);
            let service = new_service(&acceptor).await;

            assert_handshake_limit(&service, 1).await;
            initialized_tx.send(()).unwrap();

            updated_rx.recv_timeout(Duration::from_secs(5)).unwrap();

            assert_handshake_limit(&service, 1).await;

            let new_service = new_service(&acceptor).await;
            assert_handshake_limit(&new_service, 1).await;
        });
    });

    initialized_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    max_concurrent_tls_connect(3);
    updated_tx.send(()).unwrap();

    let new_thread = thread::spawn(move || {
        let runtime = LocalRuntime::new().unwrap();

        runtime.block_on(async move {
            let acceptor = Acceptor::new(config);
            let service = new_service(&acceptor).await;

            assert_handshake_limit(&service, 3).await;
        });
    });

    initialized_thread.join().unwrap();
    new_thread.join().unwrap();
}

async fn new_service(acceptor: &Acceptor) -> AcceptorService {
    <Acceptor as ServiceFactory<TcpStream>>::new_service(acceptor, ())
        .await
        .unwrap()
}

async fn assert_handshake_limit(service: &AcceptorService, limit: usize) {
    let mut cx = Context::from_waker(std::task::Waker::noop());
    let mut handshakes = Vec::new();
    let mut clients = Vec::new();

    for _ in 0..limit {
        assert_ready_ok!(<AcceptorService as Service<TcpStream>>::poll_ready(
            service, &mut cx
        ));

        let listener = net::TcpListener::bind("127.0.0.1:0").unwrap();
        let client = net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (stream, _) = listener.accept().unwrap();
        stream.set_nonblocking(true).unwrap();

        let mut handshake = Box::pin(service.call(TcpStream::from_std(stream).unwrap()));

        // The client sends no TLS data, so the handshake stays in progress.
        assert_pending!(poll!(&mut handshake).map_ok(|_| ()));

        handshakes.push(handshake);
        clients.push(client);
    }

    assert_pending!(<AcceptorService as Service<TcpStream>>::poll_ready(
        service, &mut cx
    ));

    drop(handshakes.pop());

    assert_ready_ok!(<AcceptorService as Service<TcpStream>>::poll_ready(
        service, &mut cx
    ));
}
