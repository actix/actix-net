//! Requested allocations for complete and fragmented line decoding.
use std::hint::black_box;

use actix_codec::{Decoder, LinesCodec};
use bytes::BytesMut;

#[global_allocator]
static ALLOC: divan::AllocProfiler = divan::AllocProfiler::system();
fn main() {
    divan::main();
}

#[divan::bench]
fn corpus() -> BytesMut {
    let mut src = BytesMut::from(&include_bytes!("lorem.txt")[..]);
    let mut codec = LinesCodec::new();
    while let Some(line) = codec.decode_eof(&mut src).unwrap() {
        black_box(line);
    }
    src
}

#[divan::bench(args = [64, 8192, 1048576])]
fn fragmented(size: usize) -> (BytesMut, String) {
    let mut src = BytesMut::new();
    let mut codec = LinesCodec::new();
    for _ in 0..size / 64 {
        src.extend_from_slice(black_box(&[b'a'; 64]));
        assert!(codec.decode(&mut src).unwrap().is_none());
    }
    src.extend_from_slice(b"\n");
    let line = codec.decode(&mut src).unwrap().unwrap();
    assert_eq!(line.len(), size);
    (src, line)
}
