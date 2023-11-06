#![allow(clippy::let_underscore_future)]

use std::{
    net,
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Arc,
    },
    thread,
    time::Duration,
};

use actix_rt::{net::TcpStream, time::sleep};
use actix_server::{Server, TestServer};
use actix_service::fn_service;

fn unused_addr() -> net::SocketAddr {
    TestServer::unused_addr()
}

#[test]
fn test_bind() {
    let addr = unused_addr();
    let (tx, rx) = mpsc::channel();

    let h = thread::spawn(move || {
        actix_rt::System::new().block_on(async {
            let srv = Server::build()
                .workers(1)
                .disable_signals()
                .shutdown_timeout(3600)
                .bind("test", addr, move || {
                    fn_service(|_| async { Ok::<_, ()>(()) })
                })?
                .run();

            tx.send(srv.handle()).unwrap();
            srv.await
        })
    });

    let srv = rx.recv().unwrap();

    thread::sleep(Duration::from_millis(500));

    net::TcpStream::connect(addr).unwrap();

    let _ = srv.stop(true);
    h.join().unwrap().unwrap();
}

#[test]
fn test_listen() {
    let addr = unused_addr();
    let (tx, rx) = mpsc::channel();
    let lst = net::TcpListener::bind(addr).unwrap();

    let h = thread::spawn(move || {
        actix_rt::System::new().block_on(async {
            let srv = Server::build()
                .workers(1)
                .disable_signals()
                .shutdown_timeout(3600)
                .listen("test", lst, move || {
                    fn_service(|_| async { Ok::<_, ()>(()) })
                })?
                .run();

            tx.send(srv.handle()).unwrap();
            srv.await
        })
    });

    let srv = rx.recv().unwrap();

    thread::sleep(Duration::from_millis(500));

    net::TcpStream::connect(addr).unwrap();

    let _ = srv.stop(true);
    h.join().unwrap().unwrap();
}

#[test]
fn plain_tokio_runtime() {
    let addr = unused_addr();
    let (tx, rx) = mpsc::channel();

    let h = thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        rt.block_on(async {
            let srv = Server::build()
                .workers(1)
                .disable_signals()
                .bind("test", addr, move || {
                    fn_service(|_| async { Ok::<_, ()>(()) })
                })?
                .run();

            tx.send(srv.handle()).unwrap();

            srv.await
        })
    });

    let srv = rx.recv().unwrap();

    thread::sleep(Duration::from_millis(500));
    assert!(net::TcpStream::connect(addr).is_ok());

    let _ = srv.stop(true);
    h.join().unwrap().unwrap();
}

#[test]
#[cfg(unix)]
fn test_start() {
    use std::io::Read;

    use actix_codec::{BytesCodec, Framed};
    use bytes::Bytes;
    use futures_util::sink::SinkExt;

    let addr = unused_addr();
    let (tx, rx) = mpsc::channel();

    let h = thread::spawn(move || {
        actix_rt::System::new().block_on(async {
            let srv = Server::build()
                .backlog(100)
                .disable_signals()
                .bind("test", addr, move || {
                    fn_service(|io: TcpStream| async move {
                        let mut f = Framed::new(io, BytesCodec);
                        f.send(Bytes::from_static(b"test")).await.unwrap();
                        Ok::<_, ()>(())
                    })
                })?
                .run();

            let _ = tx.send((srv.handle(), actix_rt::System::current()));

            srv.await
        })
    });

    let (srv, sys) = rx.recv().unwrap();

    let mut buf = [1u8; 4];
    let mut conn = net::TcpStream::connect(addr).unwrap();
    let _ = conn.read_exact(&mut buf);
    assert_eq!(buf, b"test"[..]);

    // pause
    let _ = srv.pause();
    thread::sleep(Duration::from_millis(200));
    let mut conn = net::TcpStream::connect(addr).unwrap();
    conn.set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    let res = conn.read_exact(&mut buf);
    assert!(res.is_err());

    // resume
    let _ = srv.resume();
    thread::sleep(Duration::from_millis(100));
    assert!(net::TcpStream::connect(addr).is_ok());
    assert!(net::TcpStream::connect(addr).is_ok());
    assert!(net::TcpStream::connect(addr).is_ok());

    let mut buf = [0u8; 4];
    let mut conn = net::TcpStream::connect(addr).unwrap();
    let _ = conn.read_exact(&mut buf);
    assert_eq!(buf, b"test"[..]);

    // stop
    let _ = srv.stop(false);
    sys.stop();
    h.join().unwrap().unwrap();

    thread::sleep(Duration::from_secs(1));
    assert!(net::TcpStream::connect(addr).is_err());
}

#[actix_rt::test]
async fn test_max_concurrent_connections() {
    // Note:
    // A TCP listener would accept connects based on it's backlog setting.
    //
    // The limit test on the other hand is only for concurrent TCP stream limiting a work
    // thread accept.

    use tokio::io::AsyncWriteExt;

    let addr = unused_addr();
    let (tx, rx) = mpsc::channel();

    let counter = Arc::new(AtomicUsize::new(0));
    let counter_clone = counter.clone();

    let max_conn = 3;

    let h = thread::spawn(move || {
        actix_rt::System::new().block_on(async {
            let srv = Server::build()
                // Set a relative higher backlog.
                .backlog(12)
                // max connection for a worker is 3.
                .max_concurrent_connections(max_conn)
                .workers(1)
                .disable_signals()
                .bind("test", addr, move || {
                    let counter = counter.clone();
                    fn_service(move |_io: TcpStream| {
                        let counter = counter.clone();
                        async move {
                            counter.fetch_add(1, Ordering::SeqCst);
                            sleep(Duration::from_secs(20)).await;
                            counter.fetch_sub(1, Ordering::SeqCst);
                            Ok::<(), ()>(())
                        }
                    })
                })?
                .run();

            let _ = tx.send((srv.handle(), actix_rt::System::current()));

            srv.await
        })
    });

    let (srv, sys) = rx.recv().unwrap();

    let mut conns = vec![];

    for _ in 0..12 {
        let conn = tokio::net::TcpStream::connect(addr).await.unwrap();
        conns.push(conn);
    }

    sleep(Duration::from_secs(5)).await;

    // counter would remain at 3 even with 12 successful connection.
    // and 9 of them remain in backlog.
    assert_eq!(max_conn, counter_clone.load(Ordering::SeqCst));

    for mut conn in conns {
        conn.shutdown().await.unwrap();
    }

    srv.stop(false).await;
    sys.stop();
    h.join().unwrap().unwrap();
}

// TODO: race-y failures detected due to integer underflow when calling Counter::total
#[actix_rt::test]
async fn test_service_restart() {
    use std::task::{Context, Poll};

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
        let num = num.clone();
        actix_rt::System::new().block_on(async {
            let srv = Server::build()
                .backlog(1)
                .disable_signals()
                .bind("addr1", addr1, move || {
                    let num = num.clone();
                    fn_factory(move || {
                        let num = num.clone();
                        async move { Ok::<_, ()>(TestService(num)) }
                    })
                })?
                .bind("addr2", addr2, move || {
                    let num2 = num2.clone();
                    fn_factory(move || {
                        let num2 = num2.clone();
                        async move { Ok::<_, ()>(TestService(num2)) }
                    })
                })?
                .workers(1)
                .run();

            let _ = tx.send(srv.handle());
            srv.await
        })
    });

    let srv = rx.recv().unwrap();

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

    let _ = srv.stop(false);
    h.join().unwrap().unwrap();
}

#[ignore] // non-deterministic on CI
#[actix_rt::test]
async fn worker_restart() {
    use actix_service::{Service, ServiceFactory};
    use futures_core::future::LocalBoxFuture;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    struct TestServiceFactory(Arc<AtomicUsize>);

    impl ServiceFactory<TcpStream> for TestServiceFactory {
        type Response = ();
        type Error = ();
        type Config = ();
        type Service = TestService;
        type InitError = ();
        type Future = LocalBoxFuture<'static, Result<Self::Service, Self::InitError>>;

        fn new_service(&self, _: Self::Config) -> Self::Future {
            let counter = self.0.fetch_add(1, Ordering::Relaxed);

            Box::pin(async move { Ok(TestService(counter)) })
        }
    }

    struct TestService(usize);

    impl Service<TcpStream> for TestService {
        type Response = ();
        type Error = ();
        type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

        actix_service::always_ready!();

        fn call(&self, stream: TcpStream) -> Self::Future {
            let counter = self.0;

            let mut stream = stream.into_std().unwrap();
            use std::io::Write;
            let str = counter.to_string();
            let buf = str.as_bytes();

            let mut written = 0;

            while written < buf.len() {
                if let Ok(n) = stream.write(&buf[written..]) {
                    written += n;
                }
            }
            stream.flush().unwrap();
            stream.shutdown(net::Shutdown::Write).unwrap();

            // force worker 2 to restart service once.
            if counter == 2 {
                panic!("panic on purpose")
            } else {
                Box::pin(async { Ok(()) })
            }
        }
    }

    let addr = unused_addr();
    let (tx, rx) = mpsc::channel();

    let counter = Arc::new(AtomicUsize::new(1));
    let h = thread::spawn(move || {
        let counter = counter.clone();
        actix_rt::System::new().block_on(async {
            let srv = Server::build()
                .disable_signals()
                .bind("addr", addr, move || TestServiceFactory(counter.clone()))?
                .workers(2)
                .run();

            let _ = tx.send(srv.handle());

            srv.await
        })
    });

    let srv = rx.recv().unwrap();

    sleep(Duration::from_secs(3)).await;

    let mut buf = [0; 8];

    // worker 1 would not restart and return it's id consistently.
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let n = stream.read(&mut buf).await.unwrap();
    let id = String::from_utf8_lossy(&buf[0..n]);
    assert_eq!("1", id);
    stream.shutdown().await.unwrap();

    // worker 2 dead after return response.
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let n = stream.read(&mut buf).await.unwrap();
    let id = String::from_utf8_lossy(&buf[0..n]);
    assert_eq!("2", id);
    stream.shutdown().await.unwrap();

    // request to worker 1
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let n = stream.read(&mut buf).await.unwrap();
    let id = String::from_utf8_lossy(&buf[0..n]);
    assert_eq!("1", id);
    stream.shutdown().await.unwrap();

    // TODO: Remove sleep if it can pass CI.
    sleep(Duration::from_secs(3)).await;

    // worker 2 restarting and work goes to worker 1.
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let n = stream.read(&mut buf).await.unwrap();
    let id = String::from_utf8_lossy(&buf[0..n]);
    assert_eq!("1", id);
    stream.shutdown().await.unwrap();

    // TODO: Remove sleep if it can pass CI.
    sleep(Duration::from_secs(3)).await;

    // worker 2 restarted but worker 1 was still the next to accept connection.
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let n = stream.read(&mut buf).await.unwrap();
    let id = String::from_utf8_lossy(&buf[0..n]);
    assert_eq!("1", id);
    stream.shutdown().await.unwrap();

    // TODO: Remove sleep if it can pass CI.
    sleep(Duration::from_secs(3)).await;

    // worker 2 accept connection again but it's id is 3.
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let n = stream.read(&mut buf).await.unwrap();
    let id = String::from_utf8_lossy(&buf[0..n]);
    assert_eq!("3", id);
    stream.shutdown().await.unwrap();

    let _ = srv.stop(false);
    h.join().unwrap().unwrap();
}

#[test]
fn no_runtime_on_init() {
    use std::{thread::sleep, time::Duration};

    let addr = unused_addr();
    let counter = Arc::new(AtomicUsize::new(0));

    let mut srv = Server::build()
        .workers(2)
        .disable_signals()
        .bind("test", addr, {
            let counter = counter.clone();
            move || {
                counter.fetch_add(1, Ordering::SeqCst);
                fn_service(|_| async { Ok::<_, ()>(()) })
            }
        })
        .unwrap()
        .run();

    fn is_send<T: Send>(_: &T) {}
    is_send(&srv);
    is_send(&srv.handle());

    sleep(Duration::from_millis(1_000));
    assert_eq!(counter.load(Ordering::SeqCst), 0);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    rt.block_on(async move {
        let _ = futures_util::poll!(&mut srv);

        // available after the first poll
        sleep(Duration::from_millis(500));
        assert_eq!(counter.load(Ordering::SeqCst), 2);

        let _ = srv.handle().stop(true);
        srv.await
    })
    .unwrap();
}
