use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::{net, thread, time};

use actix_server::ServerHandle;
use actix_service::fn_service;
use actix_utils::future::ok;
use futures_util::future::lazy;

fn unused_addr() -> net::SocketAddr {
    let addr: net::SocketAddr = "127.0.0.1:0".parse().unwrap();
    let socket = mio::net::TcpSocket::new_v4().unwrap();
    socket.bind(addr).unwrap();
    socket.set_reuseaddr(true).unwrap();
    let tcp = socket.listen(32).unwrap();
    tcp.local_addr().unwrap()
}

#[test]
fn test_bind() {
    let addr = unused_addr();
    let (tx, rx) = mpsc::channel();

    let h = thread::spawn(move || {
        let system = actix_rt::System::new();
        system.block_on(async {
            let server = ServerHandle::build()
                .workers(1)
                .disable_signals()
                .bind("test", addr, move || fn_service(|_| ok::<_, ()>(())))
                .unwrap()
                .run();
            let _ = tx.send(actix_rt::System::current());
            let _ = actix_rt::spawn(async move {
                let _ = server.await;
            });
        });

        system.run()
    });
    let sys = rx.recv().unwrap();

    thread::sleep(time::Duration::from_millis(500));
    assert!(net::TcpStream::connect(addr).is_ok());
    sys.stop();
    let _ = h.join();
}

#[test]
fn test_listen() {
    let addr = unused_addr();
    let (tx, rx) = mpsc::channel();

    let h = thread::spawn(move || {
        let lst = net::TcpListener::bind(addr).unwrap();

        actix_rt::System::new().block_on(async {
            let server = ServerHandle::build()
                .disable_signals()
                .workers(1)
                .listen("test", lst, move || {
                    fn_service(|_| async { Ok::<_, ()>(()) })
                })
                .unwrap()
                .run();

            let _ = tx.send(server.handle());

            server.await
        })
    });

    let handle = rx.recv().unwrap();

    thread::sleep(time::Duration::from_millis(500));
    assert!(net::TcpStream::connect(addr).is_ok());
    let _ = handle.stop(true);
    let _ = h.join().unwrap();
}

#[test]
#[cfg(unix)]
fn test_start() {
    use actix_codec::{BytesCodec, Framed};
    use actix_rt::net::TcpStream;
    use bytes::Bytes;
    use futures_util::sink::SinkExt;
    use std::io::Read;

    let addr = unused_addr();
    let (tx, rx) = mpsc::channel();

    let h = thread::spawn(move || {
        actix_rt::System::new().block_on(async {
            let server = ServerHandle::build()
                .backlog(100)
                .disable_signals()
                .bind("test", addr, move || {
                    fn_service(|io: TcpStream| async move {
                        let mut f = Framed::new(io, BytesCodec);
                        f.send(Bytes::from_static(b"test")).await.unwrap();
                        Ok::<_, ()>(())
                    })
                })
                .unwrap()
                .run();

            let _ = tx.send((server.handle(), actix_rt::System::current()));
            let _ = server.await;
        });
    });

    let (srv, sys) = rx.recv().unwrap();

    let mut buf = [1u8; 4];
    let mut conn = net::TcpStream::connect(addr).unwrap();
    let _ = conn.read_exact(&mut buf);
    assert_eq!(buf, b"test"[..]);

    // pause
    let _ = srv.pause();
    thread::sleep(time::Duration::from_millis(200));
    let mut conn = net::TcpStream::connect(addr).unwrap();
    conn.set_read_timeout(Some(time::Duration::from_millis(100)))
        .unwrap();
    let res = conn.read_exact(&mut buf);
    assert!(res.is_err());

    // resume
    let _ = srv.resume();
    thread::sleep(time::Duration::from_millis(100));
    assert!(net::TcpStream::connect(addr).is_ok());
    assert!(net::TcpStream::connect(addr).is_ok());
    assert!(net::TcpStream::connect(addr).is_ok());

    let mut buf = [0u8; 4];
    let mut conn = net::TcpStream::connect(addr).unwrap();
    let _ = conn.read_exact(&mut buf);
    assert_eq!(buf, b"test"[..]);

    // stop
    let _ = srv.stop(false);
    thread::sleep(time::Duration::from_millis(100));
    assert!(net::TcpStream::connect(addr).is_err());

    thread::sleep(time::Duration::from_millis(100));
    sys.stop();
    let _ = h.join();
}

#[test]
fn test_configure() {
    let addr1 = unused_addr();
    let addr2 = unused_addr();
    let addr3 = unused_addr();
    let (tx, rx) = mpsc::channel();
    let num = Arc::new(AtomicUsize::new(0));
    let num2 = num.clone();

    let h = thread::spawn(move || {
        let num = num2.clone();
        actix_rt::System::new().block_on(async {
            let server = ServerHandle::build()
                .disable_signals()
                .configure(move |cfg| {
                    let num = num.clone();
                    let lst = net::TcpListener::bind(addr3).unwrap();
                    cfg.bind("addr1", addr1)
                        .unwrap()
                        .bind("addr2", addr2)
                        .unwrap()
                        .listen("addr3", lst)
                        .apply(move |rt| {
                            let num = num.clone();
                            rt.service("addr1", fn_service(|_| ok::<_, ()>(())));
                            rt.service("addr3", fn_service(|_| ok::<_, ()>(())));
                            rt.on_start(lazy(move |_| {
                                let _ = num.fetch_add(1, Ordering::Relaxed);
                            }))
                        })
                })
                .unwrap()
                .workers(1)
                .run();

            let _ = tx.send((server.handle(), actix_rt::System::current()));
            let _ = server.await;
        });
    });
    let (server, sys) = rx.recv().unwrap();
    thread::sleep(time::Duration::from_millis(500));

    assert!(net::TcpStream::connect(addr1).is_ok());
    assert!(net::TcpStream::connect(addr2).is_ok());
    assert!(net::TcpStream::connect(addr3).is_ok());
    assert_eq!(num.load(Ordering::Relaxed), 1);
    let _ = server.stop(true);
    sys.stop();
    let _ = h.join();
}

#[actix_rt::test]
async fn test_max_concurrent_connections() {
    // Note:
    // A tcp listener would accept connects based on it's backlog setting.
    //
    // The limit test on the other hand is only for concurrent tcp stream limiting a work
    // thread accept.

    use actix_rt::net::TcpStream;
    use tokio::io::AsyncWriteExt;

    let addr = unused_addr();
    let (tx, rx) = mpsc::channel();

    let counter = Arc::new(AtomicUsize::new(0));
    let counter_clone = counter.clone();

    let max_conn = 3;

    let h = thread::spawn(move || {
        actix_rt::System::new().block_on(async {
            let server = ServerHandle::build()
                // Set a relative higher backlog.
                .backlog(12)
                // max connection for a worker is 3.
                .maxconn(max_conn)
                .workers(1)
                .disable_signals()
                .bind("test", addr, move || {
                    let counter = counter.clone();
                    fn_service(move |_io: TcpStream| {
                        let counter = counter.clone();
                        async move {
                            counter.fetch_add(1, Ordering::SeqCst);
                            actix_rt::time::sleep(time::Duration::from_secs(20)).await;
                            counter.fetch_sub(1, Ordering::SeqCst);
                            Ok::<(), ()>(())
                        }
                    })
                })?
                .run();

            let _ = tx.send((server.handle(), actix_rt::System::current()));

            server.await
        })
    });

    let (srv, sys) = rx.recv().unwrap();

    let mut conns = vec![];

    for _ in 0..12 {
        let conn = tokio::net::TcpStream::connect(addr).await.unwrap();
        conns.push(conn);
    }

    actix_rt::time::sleep(time::Duration::from_secs(5)).await;

    // counter would remain at 3 even with 12 successful connection.
    // and 9 of them remain in backlog.
    assert_eq!(max_conn, counter_clone.load(Ordering::SeqCst));

    for mut conn in conns {
        conn.shutdown().await.unwrap();
    }

    srv.stop(false).await;

    sys.stop();
    let _ = h.join().unwrap();
}

#[actix_rt::test]
async fn test_service_restart() {
    use std::task::{Context, Poll};
    use std::time::Duration;

    use actix_rt::{net::TcpStream, time::sleep};
    use actix_service::{fn_factory, Service};
    use futures_core::future::LocalBoxFuture;
    use tokio::io::AsyncWriteExt;

    struct TestService(Arc<AtomicUsize>);

    impl Service<TcpStream> for TestService {
        type Response = ();
        type Error = ();
        type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

        fn poll_ready(&self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            let TestService(ref counter) = self;
            let c = counter.fetch_add(1, Ordering::SeqCst);
            // Force the service to restart on first readiness check.
            if c > 0 {
                Poll::Ready(Ok(()))
            } else {
                Poll::Ready(Err(()))
            }
        }

        fn call(&self, _: TcpStream) -> Self::Future {
            Box::pin(async { Ok(()) })
        }
    }

    let addr1 = unused_addr();
    let addr2 = unused_addr();
    let (tx, rx) = mpsc::channel();
    let num = Arc::new(AtomicUsize::new(0));
    let num2 = Arc::new(AtomicUsize::new(0));

    let num_clone = num.clone();
    let num2_clone = num2.clone();

    let h = thread::spawn(move || {
        actix_rt::System::new().block_on(async {
            let server = ServerHandle::build()
                .backlog(1)
                .disable_signals()
                .configure(move |cfg| {
                    let num = num.clone();
                    let num2 = num2.clone();
                    cfg.bind("addr1", addr1)
                        .unwrap()
                        .bind("addr2", addr2)
                        .unwrap()
                        .apply(move |rt| {
                            let num = num.clone();
                            let num2 = num2.clone();
                            rt.service(
                                "addr1",
                                fn_factory(move || {
                                    let num = num.clone();
                                    async move { Ok::<_, ()>(TestService(num)) }
                                }),
                            );
                            rt.service(
                                "addr2",
                                fn_factory(move || {
                                    let num2 = num2.clone();
                                    async move { Ok::<_, ()>(TestService(num2)) }
                                }),
                            );
                        })
                })
                .unwrap()
                .workers(1)
                .run();

            let _ = tx.send((server.handle(), actix_rt::System::current()));
            server.await
        })
    });

    let (server, sys) = rx.recv().unwrap();

    for _ in 0..5 {
        TcpStream::connect(addr1)
            .await
            .unwrap()
            .shutdown()
            .await
            .unwrap();
        TcpStream::connect(addr2)
            .await
            .unwrap()
            .shutdown()
            .await
            .unwrap();
    }

    sleep(Duration::from_secs(3)).await;

    assert!(num_clone.load(Ordering::SeqCst) > 5);
    assert!(num2_clone.load(Ordering::SeqCst) > 5);

    sys.stop();
    let _ = server.stop(false);
    let _ = h.join().unwrap();

    let addr1 = unused_addr();
    let addr2 = unused_addr();
    let (tx, rx) = mpsc::channel();
    let num = Arc::new(AtomicUsize::new(0));
    let num2 = Arc::new(AtomicUsize::new(0));

    let num_clone = num.clone();
    let num2_clone = num2.clone();

    let h = thread::spawn(move || {
        let num = num.clone();
        actix_rt::System::new().block_on(async {
            let server = ServerHandle::build()
                .backlog(1)
                .disable_signals()
                .bind("addr1", addr1, move || {
                    let num = num.clone();
                    fn_factory(move || {
                        let num = num.clone();
                        async move { Ok::<_, ()>(TestService(num)) }
                    })
                })
                .unwrap()
                .bind("addr2", addr2, move || {
                    let num2 = num2.clone();
                    fn_factory(move || {
                        let num2 = num2.clone();
                        async move { Ok::<_, ()>(TestService(num2)) }
                    })
                })
                .unwrap()
                .workers(1)
                .run();

            let _ = tx.send((server.handle(), actix_rt::System::current()));
            server.await
        })
    });

    let (server, sys) = rx.recv().unwrap();

    for _ in 0..5 {
        TcpStream::connect(addr1)
            .await
            .unwrap()
            .shutdown()
            .await
            .unwrap();
        TcpStream::connect(addr2)
            .await
            .unwrap()
            .shutdown()
            .await
            .unwrap();
    }

    sleep(Duration::from_secs(3)).await;

    assert!(num_clone.load(Ordering::SeqCst) > 5);
    assert!(num2_clone.load(Ordering::SeqCst) > 5);

    sys.stop();
    let _ = server.stop(false);
    let _ = h.join().unwrap();
}
