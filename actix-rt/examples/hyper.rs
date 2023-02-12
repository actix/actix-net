use std::{convert::Infallible, net::SocketAddr};

use hyper::{
    service::{make_service_fn, service_fn},
    Body, Request, Response, Server,
};

async fn handle(_req: Request<Body>) -> Result<Response<Body>, Infallible> {
    Ok(Response::new(Body::from("Hello World")))
}

fn main() {
    actix_rt::System::with_tokio_rt(|| {
        // this cfg is needed when use it with io-uring, if you don't use io-uring
        // you can remove this cfg and last line in this closure
        #[cfg(not(feature = "io-uring"))]
        {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .unwrap()
        }
        // you can remove this two lines if you don't have tokio_uring feature enabled
        #[cfg(feature = "io-uring")]
        tokio_uring::Runtime::new(&tokio_uring::builder()).unwrap()
    })
    .block_on(async {
        let make_service =
            make_service_fn(|_conn| async { Ok::<_, Infallible>(service_fn(handle)) });

        let server =
            Server::bind(&SocketAddr::from(([127, 0, 0, 1], 3000))).serve(make_service);

        if let Err(err) = server.await {
            eprintln!("server error: {}", err);
        }
    })
}
