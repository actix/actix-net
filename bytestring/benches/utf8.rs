#![allow(missing_docs)]

use std::str;

use bytes::{Bytes, BytesMut};
use bytestring::ByteString;
use divan::{black_box, counter::BytesCount, AllocProfiler, Bencher};

#[global_allocator]
static ALLOC: AllocProfiler = AllocProfiler::system();

#[derive(Clone, Copy)]
enum Corpus {
    Ascii,
    AsciiLateNonAscii,
    Multibyte,
    InvalidEarly,
    InvalidLate,
}

impl Corpus {
    fn build(self, len: usize) -> Vec<u8> {
        match self {
            Corpus::Ascii => {
                const PATTERN: &[u8] = b"data: {\"text\":\"hello world 0123456789\"}\n";
                let bytes: Vec<u8> = PATTERN.iter().copied().cycle().take(len).collect();
                assert!(bytes.is_ascii());
                bytes
            }
            Corpus::AsciiLateNonAscii => {
                const SUFFIX: &[u8] = "世".as_bytes();
                assert!(len >= SUFFIX.len());
                let mut bytes = vec![b'a'; len - SUFFIX.len()];
                bytes.extend_from_slice(SUFFIX);
                assert_eq!(bytes.len(), len);
                assert!(str::from_utf8(&bytes).is_ok());
                bytes
            }
            Corpus::Multibyte => {
                const PATTERN: &[u8] = "café世界".as_bytes();
                let mut bytes = Vec::with_capacity(len);
                while bytes.len() + PATTERN.len() <= len {
                    bytes.extend_from_slice(PATTERN);
                }
                while bytes.len() + 2 <= len {
                    bytes.extend_from_slice("é".as_bytes());
                }
                if bytes.len() < len {
                    bytes.push(b'a');
                }
                assert_eq!(bytes.len(), len);
                assert!(str::from_utf8(&bytes).is_ok());
                bytes
            }
            Corpus::InvalidEarly => {
                let mut bytes = vec![b'a'; len];
                bytes[4.min(len - 1)] = 0xff;
                assert!(str::from_utf8(&bytes).is_err());
                bytes
            }
            Corpus::InvalidLate => {
                let mut bytes = Corpus::Multibyte.build(len);
                bytes[len - 1] = 0xff;
                assert!(str::from_utf8(&bytes).is_err());
                bytes
            }
        }
    }
}

fn verify_result(input: &[u8], result: Result<ByteString, str::Utf8Error>) {
    match str::from_utf8(input) {
        Ok(expected) => assert_eq!(result.expect("valid UTF-8").as_str(), expected),
        Err(expected) => assert_eq!(result.expect_err("invalid UTF-8"), expected),
    }
}

fn bench_from_utf8(b: Bencher<'_, '_>, corpus: Corpus, size: usize) {
    let raw = corpus.build(size);
    let bytes = Bytes::from(raw.clone());

    verify_result(
        &raw,
        ByteString::from_utf8(bytes.clone()).map_err(|err| err.utf8_error()),
    );

    b.counter(BytesCount::new(size))
        .with_inputs(|| bytes.clone())
        .bench_values(|buf| ByteString::from_utf8(black_box(buf)));
}

fn bench_try_from_bytes(b: Bencher<'_, '_>, corpus: Corpus, size: usize) {
    let raw = corpus.build(size);
    let bytes = Bytes::from(raw.clone());

    verify_result(&raw, ByteString::try_from(bytes.clone()));

    b.counter(BytesCount::new(size))
        .with_inputs(|| bytes.clone())
        .bench_values(|buf| ByteString::try_from(black_box(buf)));
}

fn bench_try_from_slice(b: Bencher<'_, '_>, corpus: Corpus, size: usize) {
    let raw = corpus.build(size);

    verify_result(&raw, ByteString::try_from(raw.as_slice()));

    b.counter(BytesCount::new(size))
        .bench(|| ByteString::try_from(black_box(raw.as_slice())));
}

fn bench_try_from_vec(b: Bencher<'_, '_>, corpus: Corpus, size: usize) {
    let raw = corpus.build(size);

    verify_result(&raw, ByteString::try_from(raw.clone()));

    b.counter(BytesCount::new(size))
        .with_inputs(|| raw.clone())
        .bench_values(|vec| ByteString::try_from(black_box(vec)));
}

fn bench_try_from_bytes_mut(b: Bencher<'_, '_>, corpus: Corpus, size: usize) {
    let raw = corpus.build(size);

    verify_result(&raw, ByteString::try_from(BytesMut::from(raw.as_slice())));

    b.counter(BytesCount::new(size))
        .with_inputs(|| BytesMut::from(raw.as_slice()))
        .bench_values(|buf| ByteString::try_from(black_box(buf)));
}

macro_rules! bench_sizes {
    ($name:ident, $bench:ident, $corpus:path, [$small:literal, $medium:literal, $large:literal, $largest:literal]) => {
        bench_sizes!(@case $name, $bench, $corpus, 16, $small);
        bench_sizes!(@case $name, $bench, $corpus, 64, $medium);
        bench_sizes!(@case $name, $bench, $corpus, 1024, $large);
        bench_sizes!(@case $name, $bench, $corpus, 65_536, $largest);
    };
    (@case $name:ident, $bench:ident, $corpus:path, $size:literal, $samples:literal) => {
        const _: () = {
            #[divan::bench(args = [$size], sample_size = $samples)]
            fn $name(b: Bencher<'_, '_>, size: usize) {
                $bench(b, $corpus, size);
            }
        };
    };
}

bench_sizes!(
    from_utf8_ascii,
    bench_from_utf8,
    Corpus::Ascii,
    [128, 128, 32, 1]
);
bench_sizes!(
    from_utf8_ascii_late_non_ascii,
    bench_from_utf8,
    Corpus::AsciiLateNonAscii,
    [64, 64, 32, 1]
);
bench_sizes!(
    from_utf8_multibyte,
    bench_from_utf8,
    Corpus::Multibyte,
    [64, 64, 4, 1]
);
bench_sizes!(
    from_utf8_invalid_early,
    bench_from_utf8,
    Corpus::InvalidEarly,
    [128, 32, 32, 32]
);
bench_sizes!(
    from_utf8_invalid_late,
    bench_from_utf8,
    Corpus::InvalidLate,
    [64, 16, 1, 1]
);

bench_sizes!(
    try_from_bytes_ascii,
    bench_try_from_bytes,
    Corpus::Ascii,
    [128, 128, 32, 1]
);
bench_sizes!(
    try_from_bytes_ascii_late_non_ascii,
    bench_try_from_bytes,
    Corpus::AsciiLateNonAscii,
    [64, 64, 32, 1]
);
bench_sizes!(
    try_from_bytes_multibyte,
    bench_try_from_bytes,
    Corpus::Multibyte,
    [64, 64, 4, 1]
);
bench_sizes!(
    try_from_bytes_invalid_early,
    bench_try_from_bytes,
    Corpus::InvalidEarly,
    [64, 32, 32, 32]
);
bench_sizes!(
    try_from_bytes_invalid_late,
    bench_try_from_bytes,
    Corpus::InvalidLate,
    [64, 16, 1, 1]
);

bench_sizes!(
    try_from_slice_ascii,
    bench_try_from_slice,
    Corpus::Ascii,
    [32, 32, 8, 1]
);
bench_sizes!(
    try_from_slice_ascii_late_non_ascii,
    bench_try_from_slice,
    Corpus::AsciiLateNonAscii,
    [16, 16, 8, 1]
);
bench_sizes!(
    try_from_slice_multibyte,
    bench_try_from_slice,
    Corpus::Multibyte,
    [16, 16, 4, 1]
);
bench_sizes!(
    try_from_slice_invalid_early,
    bench_try_from_slice,
    Corpus::InvalidEarly,
    [128, 32, 32, 32]
);
bench_sizes!(
    try_from_slice_invalid_late,
    bench_try_from_slice,
    Corpus::InvalidLate,
    [64, 16, 1, 1]
);

bench_sizes!(
    try_from_vec_ascii,
    bench_try_from_vec,
    Corpus::Ascii,
    [128, 128, 32, 1]
);
bench_sizes!(
    try_from_vec_ascii_late_non_ascii,
    bench_try_from_vec,
    Corpus::AsciiLateNonAscii,
    [64, 64, 16, 1]
);
bench_sizes!(
    try_from_vec_multibyte,
    bench_try_from_vec,
    Corpus::Multibyte,
    [64, 64, 4, 1]
);
bench_sizes!(
    try_from_vec_invalid_early,
    bench_try_from_vec,
    Corpus::InvalidEarly,
    [64, 32, 32, 16]
);
bench_sizes!(
    try_from_vec_invalid_late,
    bench_try_from_vec,
    Corpus::InvalidLate,
    [32, 8, 1, 1]
);

bench_sizes!(
    try_from_bytes_mut_ascii,
    bench_try_from_bytes_mut,
    Corpus::Ascii,
    [128, 64, 32, 1]
);
bench_sizes!(
    try_from_bytes_mut_ascii_late_non_ascii,
    bench_try_from_bytes_mut,
    Corpus::AsciiLateNonAscii,
    [64, 32, 16, 1]
);
bench_sizes!(
    try_from_bytes_mut_multibyte,
    bench_try_from_bytes_mut,
    Corpus::Multibyte,
    [64, 32, 4, 1]
);
bench_sizes!(
    try_from_bytes_mut_invalid_early,
    bench_try_from_bytes_mut,
    Corpus::InvalidEarly,
    [64, 32, 16, 16]
);
bench_sizes!(
    try_from_bytes_mut_invalid_late,
    bench_try_from_bytes_mut,
    Corpus::InvalidLate,
    [32, 8, 1, 1]
);

fn main() {
    divan::main();
}
