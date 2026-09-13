//! Compare default buffers with explicit 8 KiB buffers on idle and active transports.
use std::{future::poll_fn, hint::black_box, pin::Pin};

use actix_codec::{AsyncRead, AsyncWrite, Framed, FramedParts, LinesCodec};
use bytes::BytesMut;
use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion};
use futures_sink::Sink;

fn framed<T>(io: T, preallocated: bool) -> Framed<T, LinesCodec> {
    if preallocated {
        let mut parts = FramedParts::new(io, LinesCodec::new());
        parts.read_buf = BytesMut::with_capacity(8192);
        parts.write_buf = BytesMut::with_capacity(8192);
        Framed::from_parts(parts)
    } else {
        Framed::new(io, LinesCodec::new())
    }
}

async fn send<T: AsyncWrite + Unpin>(framed: &mut Framed<T, LinesCodec>, line: &str) {
    poll_fn(|cx| <Framed<T, LinesCodec> as Sink<&str>>::poll_ready(Pin::new(&mut *framed), cx))
        .await
        .unwrap();
    Pin::new(&mut *framed).write(line).unwrap();
    poll_fn(|cx| Pin::new(&mut *framed).flush::<&str>(cx))
        .await
        .unwrap();
}

async fn exchange<T: AsyncRead + AsyncWrite + Unpin>(
    client: &mut Framed<T, LinesCodec>,
    server: &mut Framed<T, LinesCodec>,
    input: &str,
    rounds: usize,
) {
    tokio::join!(
        async {
            for _ in 0..rounds {
                send(client, input).await;
                let line = poll_fn(|cx| Pin::new(&mut *client).next_item(cx))
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(line, input);
                black_box(line);
            }
        },
        async {
            for _ in 0..rounds {
                let line = poll_fn(|cx| Pin::new(&mut *server).next_item(cx))
                    .await
                    .unwrap()
                    .unwrap();
                send(server, &line).await;
            }
        }
    );
}

fn benchmarks(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let mut construct = c.benchmark_group("framed construct");
    for preallocated in [true, false] {
        construct.bench_function(
            if preallocated {
                "preallocated"
            } else {
                "default"
            },
            |b| {
                b.iter(|| black_box(framed(tokio::io::empty(), preallocated)));
            },
        );
    }
    construct.finish();

    let mut first_use = c.benchmark_group("framed first use");
    for preallocated in [true, false] {
        let policy = if preallocated {
            "preallocated"
        } else {
            "default"
        };
        first_use.bench_function(format!("write/{policy}"), |b| {
            b.iter_batched(
                || framed(tokio::io::sink(), preallocated),
                |mut transport| {
                    Pin::new(&mut transport).write(black_box("hello")).unwrap();
                    black_box(transport)
                },
                BatchSize::LargeInput,
            );
        });
        first_use.bench_function(format!("read/{policy}"), |b| {
            b.iter_batched(
                || framed(&b"hello\n"[..], preallocated),
                |mut transport| {
                    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
                    assert!(matches!(Pin::new(&mut transport).next_item(&mut cx), std::task::Poll::Ready(Some(Ok(line))) if line == "hello"));
                    black_box(transport)
                },
                BatchSize::LargeInput,
            );
        });
    }
    first_use.finish();

    for (size, rounds) in [(64, 1), (64, 64), (8192, 1), (8192, 64)] {
        let input = "x".repeat(size);
        let mut group = c.benchmark_group(format!("framed duplex/{size}/{rounds}"));
        for preallocated in [true, false] {
            group.bench_with_input(
                BenchmarkId::from_parameter(if preallocated {
                    "preallocated"
                } else {
                    "default"
                }),
                &preallocated,
                |b, &preallocated| {
                    b.iter(|| {
                        rt.block_on(async {
                            let (client, server) = tokio::io::duplex(64);
                            exchange(
                                &mut framed(client, preallocated),
                                &mut framed(server, preallocated),
                                black_box(&input),
                                rounds,
                            )
                            .await;
                        })
                    });
                },
            );
        }
        group.finish();
    }

    let input = "x".repeat(64);
    let listener = rt
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let addr = listener.local_addr().unwrap();
    for rounds in [1, 64] {
        let mut group = c.benchmark_group(format!("framed tcp warm/{rounds}"));
        for preallocated in [true, false] {
            group.bench_function(
                if preallocated {
                    "preallocated"
                } else {
                    "default"
                },
                |b| {
                    let (client, server) = rt.block_on(async {
                        let (client, server) =
                            tokio::join!(tokio::net::TcpStream::connect(addr), listener.accept());
                        let client = client.unwrap();
                        let (server, _) = server.unwrap();
                        client.set_nodelay(true).unwrap();
                        server.set_nodelay(true).unwrap();
                        (client, server)
                    });
                    let mut client = framed(client, preallocated);
                    let mut server = framed(server, preallocated);
                    b.iter(|| {
                        rt.block_on(exchange(
                            &mut client,
                            &mut server,
                            black_box(&input),
                            rounds,
                        ))
                    });
                },
            );
        }
        group.finish();
    }
}
criterion_group!(benches, benchmarks);
criterion_main!(benches);
