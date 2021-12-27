use std::net;

use actix_rt::net::TcpStream;
use actix_server::{Server, TestServer};
use actix_service::fn_service;
use bytes::BytesMut;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

macro_rules! await_timeout_ms {
    ($fut:expr, $limit:expr) => {
        ::actix_rt::time::timeout(::std::time::Duration::from_millis($limit), $fut)
            .await
            .unwrap()
            .unwrap();
    };
}

#[tokio::test]
async fn testing_server_echo() {
    let srv = TestServer::start(|| {
        fn_service(move |mut stream: TcpStream| async move {
            let mut size = 0;
            let mut buf = BytesMut::new();

            match stream.read_buf(&mut buf).await {
                Ok(0) => return Err(()),

                Ok(bytes_read) => {
                    stream.write_all(&buf[size..]).await.unwrap();
                    size += bytes_read;
                }

                Err(_) => return Err(()),
            }

            Ok((buf.freeze(), size))
        })
    });

    let mut conn = srv.connect().unwrap();

    await_timeout_ms!(conn.write_all(b"test"), 200);

    let mut buf = Vec::new();
    await_timeout_ms!(conn.read_to_end(&mut buf), 200);

    assert_eq!(&buf, b"test".as_ref());
}

#[tokio::test]
async fn new_with_builder() {
    let alt_addr = TestServer::unused_addr();

    let srv = TestServer::start_with_builder(
        Server::build()
            .bind("alt", alt_addr, || {
                fn_service(|_| async { Ok::<_, ()>(()) })
            })
            .unwrap(),
        || {
            fn_service(|mut sock: TcpStream| async move {
                let mut buf = [0u8; 16];
                sock.read_exact(&mut buf).await
            })
        },
    );

    // connect to test server
    srv.connect().unwrap();

    // connect to alt service defined in custom ServerBuilder
    TcpStream::from_std(net::TcpStream::connect(alt_addr).unwrap()).unwrap();
}
