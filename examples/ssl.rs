use std::io;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use actix_rt::System;
use actix_server::{ssl, Server};
use actix_service::NewService;
use futures_util::future;
use openssl::ssl::{SslAcceptor, SslFiletype, SslMethod};

#[derive(Debug)]
struct ServiceState {
    num: Arc<AtomicUsize>,
}

fn main() -> io::Result<()> {
    let sys = System::new("test");

    // load ssl keys
    let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
    builder
        .set_private_key_file("./examples/key.pem", SslFiletype::PEM)
        .unwrap();
    builder
        .set_certificate_chain_file("./examples/cert.pem")
        .unwrap();

    let num = Arc::new(AtomicUsize::new(0));
    let openssl = ssl::OpensslAcceptor::new(builder.build());

    // server start mutiple workers, it runs supplied `Fn` in each worker.
    Server::build()
        .bind("test-ssl", "0.0.0.0:8443", move || {
            let num = num.clone();

            // configure service
            openssl
                .clone()
                .map_err(|e| println!("Openssl error: {}", e))
                .and_then(move |_| {
                    let num = num.fetch_add(1, Ordering::Relaxed);
                    println!("got ssl connection {:?}", num);
                    future::ok(())
                })
        })?
        .start();

    sys.run()
}
