use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use actix_codec::BytesCodec;
use actix_server_config::Io;
use actix_service::{apply_fn_factory, service_fn, Service};
use actix_testing::{self as test, TestServer};
use futures::future::ok;
use tokio_net::tcp::TcpStream;
use tokio_timer::delay_for;

use actix_ioframe::{Builder, Connect};

struct State;

#[test]
fn test_disconnect() -> std::io::Result<()> {
    let disconnect = Arc::new(AtomicBool::new(false));
    let disconnect1 = disconnect.clone();

    let srv = TestServer::with(move || {
        let disconnect1 = disconnect1.clone();

        apply_fn_factory(
            Builder::new()
                .factory(service_fn(|conn: Connect<_>| {
                    ok(conn.codec(BytesCodec).state(State))
                }))
                .disconnect(move |_, _| {
                    disconnect1.store(true, Ordering::Relaxed);
                })
                .finish(service_fn(|_t| ok(None))),
            |io: Io<TcpStream>, srv| srv.call(io.into_parts().0),
        )
    });

    let mut client = Builder::new()
        .service(|conn: Connect<_>| {
            let conn = conn.codec(BytesCodec).state(State);
            conn.sink().close();
            ok(conn)
        })
        .finish(service_fn(|_t| ok(None)));

    let conn = test::block_on(
        actix_connect::default_connector()
            .call(actix_connect::Connect::with(String::new(), srv.addr())),
    )
    .unwrap();

    test::block_on(client.call(conn.into_parts().0)).unwrap();
    let _ = test::block_on(delay_for(Duration::from_millis(100)));
    assert!(disconnect.load(Ordering::Relaxed));

    Ok(())
}
