#![allow(missing_docs)]

use std::hint::black_box;

use bytes::BytesMut;
use criterion::{criterion_group, criterion_main, Criterion};

const INPUT: &[u8] = include_bytes!("./lorem.txt");
const PARTIAL_CHUNK: [u8; 64] = [b'a'; 64];
const PARTIAL_CHUNKS: usize = 128;

fn bench_lines_codec(c: &mut Criterion) {
    let mut decode_group = c.benchmark_group("lines decode");

    decode_group.bench_function("actix", |b| {
        b.iter(|| {
            use actix_codec::Decoder as _;

            let mut codec = actix_codec::LinesCodec::default();
            let mut buf = BytesMut::from(INPUT);
            while let Some(line) = codec.decode_eof(&mut buf).unwrap() {
                black_box(line);
            }
        });
    });

    decode_group.bench_function("tokio", |b| {
        b.iter(|| {
            use tokio_util::codec::Decoder as _;

            let mut codec = tokio_util::codec::LinesCodec::new();
            let mut buf = BytesMut::from(INPUT);
            while let Some(line) = codec.decode_eof(&mut buf).unwrap() {
                black_box(line);
            }
        });
    });

    decode_group.finish();

    let mut partial_decode_group = c.benchmark_group("lines decode partial");

    partial_decode_group.bench_function("actix", |b| {
        b.iter(|| {
            use actix_codec::Decoder as _;

            let mut codec = actix_codec::LinesCodec::default();
            let mut buf = BytesMut::with_capacity(PARTIAL_CHUNK.len() * PARTIAL_CHUNKS + 1);

            for _ in 0..PARTIAL_CHUNKS {
                buf.extend_from_slice(&PARTIAL_CHUNK);
                assert!(codec.decode(&mut buf).unwrap().is_none());
            }

            buf.extend_from_slice(b"\n");
            assert!(codec.decode(&mut buf).unwrap().is_some());
        });
    });

    partial_decode_group.bench_function("tokio", |b| {
        b.iter(|| {
            use tokio_util::codec::Decoder as _;

            let mut codec = tokio_util::codec::LinesCodec::new();
            let mut buf = BytesMut::with_capacity(PARTIAL_CHUNK.len() * PARTIAL_CHUNKS + 1);

            for _ in 0..PARTIAL_CHUNKS {
                buf.extend_from_slice(&PARTIAL_CHUNK);
                assert!(codec.decode(&mut buf).unwrap().is_none());
            }

            buf.extend_from_slice(b"\n");
            assert!(codec.decode(&mut buf).unwrap().is_some());
        });
    });

    partial_decode_group.finish();

    let mut encode_group = c.benchmark_group("lines encode");

    encode_group.bench_function("actix", |b| {
        b.iter(|| {
            use actix_codec::Encoder as _;

            let mut codec = actix_codec::LinesCodec::default();
            let mut buf = BytesMut::new();
            codec.encode("123", &mut buf).unwrap();
        });
    });

    encode_group.bench_function("tokio", |b| {
        b.iter(|| {
            use tokio_util::codec::Encoder as _;

            let mut codec = tokio_util::codec::LinesCodec::new();
            let mut buf = BytesMut::new();
            codec.encode("123", &mut buf).unwrap();
        });
    });

    encode_group.finish();
}

fn bench_fragment_sizes(c: &mut Criterion) {
    use actix_codec::{Decoder as _, LinesCodec};

    for size in [64, 8192, 1048576] {
        let input = vec![b'a'; size];

        let mut group = c.benchmark_group(format!("lines fragments/{size}"));

        for chunk in [64, 1024] {
            group.bench_function(format!("chunk_{chunk}"), |b| {
                b.iter(|| {
                    let mut codec = LinesCodec::new();
                    let mut src = BytesMut::with_capacity(size + 1);

                    for part in input.chunks(chunk) {
                        src.extend_from_slice(black_box(part));
                        assert!(codec.decode(&mut src).unwrap().is_none());
                    }

                    src.extend_from_slice(b"\n");

                    black_box(codec.decode(&mut src).unwrap().unwrap());
                })
            });
        }

        group.finish();
    }
}

criterion_group!(benches, bench_lines_codec, bench_fragment_sizes);
criterion_main!(benches);
