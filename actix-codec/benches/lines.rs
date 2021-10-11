use bytes::BytesMut;
use criterion::{criterion_group, criterion_main, Criterion};

const INPUT: &[u8] = include_bytes!("./lorem.txt");

fn bench_lines_codec(c: &mut Criterion) {
    let mut decode_group = c.benchmark_group("lines decode");

    decode_group.bench_function("actix", |b| {
        b.iter(|| {
            use actix_codec::Decoder as _;

            let mut codec = actix_codec::LinesCodec::default();
            let mut buf = BytesMut::from(INPUT);
            while let Ok(Some(_bytes)) = codec.decode_eof(&mut buf) {}
        });
    });

    decode_group.bench_function("tokio", |b| {
        b.iter(|| {
            use tokio_util::codec::Decoder as _;

            let mut codec = tokio_util::codec::LinesCodec::new();
            let mut buf = BytesMut::from(INPUT);
            while let Ok(Some(_bytes)) = codec.decode_eof(&mut buf) {}
        });
    });

    decode_group.finish();

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

criterion_group!(benches, bench_lines_codec);
criterion_main!(benches);
