#![cfg(feature = "connect")]

use std::{
    io,
    net::{IpAddr, Ipv4Addr},
};

use actix_codec::{BytesCodec, Framed};
use actix_rt::net::TcpStream;
use actix_server::TestServer;
use actix_service::{fn_service, Service, ServiceFactory};
use actix_tls::connect::{ConnectError, ConnectInfo, Connection, Connector, Host};
use bytes::Bytes;
use futures_util::sink::SinkExt as _;

#[cfg(feature = "openssl")]
#[actix_rt::test]
async fn test_string() {
    let srv = TestServer::start(|| {
        fn_service(|io: TcpStream| async {
            let mut framed = Framed::new(io, BytesCodec);
            framed.send(Bytes::from_static(b"test")).await?;
            Ok::<_, io::Error>(())
        })
    });

    let connector = Connector::default().service();
    let addr = format!("localhost:{}", srv.port());
    let con = connector.call(addr.into()).await.unwrap();
    assert_eq!(con.peer_addr().unwrap(), srv.addr());
}

#[cfg(feature = "rustls-0_22")]
#[actix_rt::test]
async fn test_rustls_string() {
    let srv = TestServer::start(|| {
        fn_service(|io: TcpStream| async {
            let mut framed = Framed::new(io, BytesCodec);
            framed.send(Bytes::from_static(b"test")).await?;
            Ok::<_, io::Error>(())
        })
    });

    let conn = Connector::default().service();
    let addr = format!("localhost:{}", srv.port());
    let con = conn.call(addr.into()).await.unwrap();
    assert_eq!(con.peer_addr().unwrap(), srv.addr());
}

#[actix_rt::test]
async fn test_static_str() {
    let srv = TestServer::start(|| {
        fn_service(|io: TcpStream| async {
            let mut framed = Framed::new(io, BytesCodec);
            framed.send(Bytes::from_static(b"test")).await?;
            Ok::<_, io::Error>(())
        })
    });

    let info = ConnectInfo::with_addr("10", srv.addr());
    let connector = Connector::default().service();
    let conn = connector.call(info).await.unwrap();
    assert_eq!(conn.peer_addr().unwrap(), srv.addr());

    let info = ConnectInfo::new(srv.host().to_owned());
    let connector = Connector::default().service();
    let conn = connector.call(info).await;
    assert!(conn.is_err());
}

#[actix_rt::test]
async fn service_factory() {
    pub fn default_connector_factory<T: Host + 'static>() -> impl ServiceFactory<
        ConnectInfo<T>,
        Config = (),
        Response = Connection<T, TcpStream>,
        Error = ConnectError,
        InitError = (),
    > {
        Connector::default()
    }

    let srv = TestServer::start(|| {
        fn_service(|io: TcpStream| async {
            let mut framed = Framed::new(io, BytesCodec);
            framed.send(Bytes::from_static(b"test")).await?;
            Ok::<_, io::Error>(())
        })
    });

    let info = ConnectInfo::with_addr("10", srv.addr());
    let factory = default_connector_factory();
    let connector = factory.new_service(()).await.unwrap();
    let con = connector.call(info).await;
    assert_eq!(con.unwrap().peer_addr().unwrap(), srv.addr());
}

#[cfg(all(feature = "openssl", feature = "uri"))]
#[actix_rt::test]
async fn test_openssl_uri() {
    use std::convert::TryFrom;

    let srv = TestServer::start(|| {
        fn_service(|io: TcpStream| async {
            let mut framed = Framed::new(io, BytesCodec);
            framed.send(Bytes::from_static(b"test")).await?;
            Ok::<_, io::Error>(())
        })
    });

    let connector = Connector::default().service();
    let addr = http_0_2::Uri::try_from(format!("https://localhost:{}", srv.port())).unwrap();
    let con = connector.call(addr.into()).await.unwrap();
    assert_eq!(con.peer_addr().unwrap(), srv.addr());
}

#[cfg(all(feature = "rustls-0_22", feature = "uri"))]
#[actix_rt::test]
async fn test_rustls_uri_http1() {
    let srv = TestServer::start(|| {
        fn_service(|io: TcpStream| async {
            let mut framed = Framed::new(io, BytesCodec);
            framed.send(Bytes::from_static(b"test")).await?;
            Ok::<_, io::Error>(())
        })
    });

    let conn = Connector::default().service();
    let addr = http_1::Uri::try_from(format!("https://localhost:{}", srv.port())).unwrap();
    let con = conn.call(addr.into()).await.unwrap();
    assert_eq!(con.peer_addr().unwrap(), srv.addr());
}

#[cfg(all(feature = "rustls-0_22", feature = "uri"))]
#[actix_rt::test]
async fn test_rustls_uri() {
    use std::convert::TryFrom;

    let srv = TestServer::start(|| {
        fn_service(|io: TcpStream| async {
            let mut framed = Framed::new(io, BytesCodec);
            framed.send(Bytes::from_static(b"test")).await?;
            Ok::<_, io::Error>(())
        })
    });

    let conn = Connector::default().service();
    let addr = http_1::Uri::try_from(format!("https://localhost:{}", srv.port())).unwrap();
    let con = conn.call(addr.into()).await.unwrap();
    assert_eq!(con.peer_addr().unwrap(), srv.addr());
}

#[actix_rt::test]
async fn test_local_addr() {
    let srv = TestServer::start(|| {
        fn_service(|io: TcpStream| async {
            let mut framed = Framed::new(io, BytesCodec);
            framed.send(Bytes::from_static(b"test")).await?;
            Ok::<_, io::Error>(())
        })
    });

    // if you've arrived here because of a failing test on macOS run this in your terminal:
    // sudo ifconfig lo0 alias 127.0.0.3

    let conn = Connector::default().service();
    let local = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 3));

    let (con, _) = conn
        .call(ConnectInfo::with_addr("10", srv.addr()).set_local_addr(local))
        .await
        .unwrap()
        .into_parts();

    assert_eq!(con.local_addr().unwrap().ip(), local)
}
