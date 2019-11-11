use actix_codec::{BytesCodec, Framed};
use actix_server_config::Io;
use actix_service::{service_fn, NewService, Service};
use actix_testing::{self as test, TestServer};
use bytes::Bytes;
use futures::{future::lazy, Future, Sink};
use http::{HttpTryFrom, Uri};
use trust_dns_resolver::config::{ResolverConfig, ResolverOpts};

use actix_connect::{default_connector, Connect};

#[cfg(feature = "ssl")]
#[test]
fn test_string() {
    let srv = TestServer::with(|| {
        service_fn(|io: Io<tokio_tcp::TcpStream>| {
            Framed::new(io.into_parts().0, BytesCodec)
                .send(Bytes::from_static(b"test"))
                .then(|_| Ok::<_, ()>(()))
        })
    });

    let mut conn = default_connector();
    let addr = format!("localhost:{}", srv.port());
    let con = test::call_service(&mut conn, addr.into());
    assert_eq!(con.peer_addr().unwrap(), srv.addr());
}

#[cfg(feature = "rust-tls")]
#[test]
fn test_rustls_string() {
    let srv = TestServer::with(|| {
        service_fn(|io: Io<tokio_tcp::TcpStream>| {
            Framed::new(io.into_parts().0, BytesCodec)
                .send(Bytes::from_static(b"test"))
                .then(|_| Ok::<_, ()>(()))
        })
    });

    let mut conn = default_connector();
    let addr = format!("localhost:{}", srv.port());
    let con = test::call_service(&mut conn, addr.into());
    assert_eq!(con.peer_addr().unwrap(), srv.addr());
}

#[test]
fn test_static_str() {
    let srv = TestServer::with(|| {
        service_fn(|io: Io<tokio_tcp::TcpStream>| {
            Framed::new(io.into_parts().0, BytesCodec)
                .send(Bytes::from_static(b"test"))
                .then(|_| Ok::<_, ()>(()))
        })
    });

    let resolver = test::block_on(lazy(
        || Ok::<_, ()>(actix_connect::start_default_resolver()),
    ))
    .unwrap();

    let mut conn = test::block_on(lazy(|| {
        Ok::<_, ()>(actix_connect::new_connector(resolver.clone()))
    }))
    .unwrap();

    let con = test::block_on(conn.call(Connect::with("10", srv.addr()))).unwrap();
    assert_eq!(con.peer_addr().unwrap(), srv.addr());

    let connect = Connect::new(srv.host().to_owned());
    let mut conn =
        test::block_on(lazy(|| Ok::<_, ()>(actix_connect::new_connector(resolver)))).unwrap();
    let con = test::block_on(conn.call(connect));
    assert!(con.is_err());
}

#[test]
fn test_new_service() {
    let srv = TestServer::with(|| {
        service_fn(|io: Io<tokio_tcp::TcpStream>| {
            Framed::new(io.into_parts().0, BytesCodec)
                .send(Bytes::from_static(b"test"))
                .then(|_| Ok::<_, ()>(()))
        })
    });

    let resolver = test::block_on(lazy(|| {
        Ok::<_, ()>(actix_connect::start_resolver(
            ResolverConfig::default(),
            ResolverOpts::default(),
        ))
    }))
    .unwrap();
    let factory = test::block_on(lazy(|| {
        Ok::<_, ()>(actix_connect::new_connector_factory(resolver))
    }))
    .unwrap();

    let mut conn = test::block_on(factory.new_service(&())).unwrap();
    let con = test::block_on(conn.call(Connect::with("10", srv.addr()))).unwrap();
    assert_eq!(con.peer_addr().unwrap(), srv.addr());
}

#[cfg(feature = "ssl")]
#[test]
fn test_uri() {
    let srv = TestServer::with(|| {
        service_fn(|io: Io<tokio_tcp::TcpStream>| {
            Framed::new(io.into_parts().0, BytesCodec)
                .send(Bytes::from_static(b"test"))
                .then(|_| Ok::<_, ()>(()))
        })
    });

    let mut conn = default_connector();
    let addr = Uri::try_from(format!("https://localhost:{}", srv.port())).unwrap();
    let con = test::call_service(&mut conn, addr.into());
    assert_eq!(con.peer_addr().unwrap(), srv.addr());
}

#[cfg(feature = "rust-tls")]
#[test]
fn test_rustls_uri() {
    let srv = TestServer::with(|| {
        service_fn(|io: Io<tokio_tcp::TcpStream>| {
            Framed::new(io.into_parts().0, BytesCodec)
                .send(Bytes::from_static(b"test"))
                .then(|_| Ok::<_, ()>(()))
        })
    });

    let mut conn = default_connector();
    let addr = Uri::try_from(format!("https://localhost:{}", srv.port())).unwrap();
    let con = test::call_service(&mut conn, addr.into());
    assert_eq!(con.peer_addr().unwrap(), srv.addr());
}
