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
    ($name:ident, [$n16:literal, $n64:literal, $n1024:literal, $n65536:literal]) => {
        bench_sizes!(@case $name, 16, $n16);
        bench_sizes!(@case $name, 64, $n64);
        bench_sizes!(@case $name, 1024, $n1024);
        bench_sizes!(@case $name, 65_536, $n65536);
    };
    (@case $name:ident, $size:literal, $samples:literal) => {
        const _: () = {
            #[divan::bench(name = stringify!($name), args = [$size], sample_size = $samples)]
            fn case(b: Bencher<'_, '_>, size: usize) {
                $name(b, size);
            }
        };
    };
}

bench_sizes!(from_utf8_ascii, [128, 128, 32, 1]);
fn from_utf8_ascii(b: Bencher<'_, '_>, size: usize) {
    bench_from_utf8(b, Corpus::Ascii, size);
}

bench_sizes!(from_utf8_ascii_late_non_ascii, [64, 64, 32, 1]);
fn from_utf8_ascii_late_non_ascii(b: Bencher<'_, '_>, size: usize) {
    bench_from_utf8(b, Corpus::AsciiLateNonAscii, size);
}

bench_sizes!(from_utf8_multibyte, [64, 64, 4, 1]);
fn from_utf8_multibyte(b: Bencher<'_, '_>, size: usize) {
    bench_from_utf8(b, Corpus::Multibyte, size);
}

bench_sizes!(from_utf8_invalid_early, [128, 32, 32, 32]);
fn from_utf8_invalid_early(b: Bencher<'_, '_>, size: usize) {
    bench_from_utf8(b, Corpus::InvalidEarly, size);
}

bench_sizes!(from_utf8_invalid_late, [64, 16, 1, 1]);
fn from_utf8_invalid_late(b: Bencher<'_, '_>, size: usize) {
    bench_from_utf8(b, Corpus::InvalidLate, size);
}

bench_sizes!(try_from_bytes_ascii, [128, 128, 32, 1]);
fn try_from_bytes_ascii(b: Bencher<'_, '_>, size: usize) {
    bench_try_from_bytes(b, Corpus::Ascii, size);
}

bench_sizes!(try_from_bytes_ascii_late_non_ascii, [64, 64, 32, 1]);
fn try_from_bytes_ascii_late_non_ascii(b: Bencher<'_, '_>, size: usize) {
    bench_try_from_bytes(b, Corpus::AsciiLateNonAscii, size);
}

bench_sizes!(try_from_bytes_multibyte, [64, 64, 4, 1]);
fn try_from_bytes_multibyte(b: Bencher<'_, '_>, size: usize) {
    bench_try_from_bytes(b, Corpus::Multibyte, size);
}

bench_sizes!(try_from_bytes_invalid_early, [64, 32, 32, 32]);
fn try_from_bytes_invalid_early(b: Bencher<'_, '_>, size: usize) {
    bench_try_from_bytes(b, Corpus::InvalidEarly, size);
}

bench_sizes!(try_from_bytes_invalid_late, [64, 16, 1, 1]);
fn try_from_bytes_invalid_late(b: Bencher<'_, '_>, size: usize) {
    bench_try_from_bytes(b, Corpus::InvalidLate, size);
}

bench_sizes!(try_from_slice_ascii, [32, 32, 8, 1]);
fn try_from_slice_ascii(b: Bencher<'_, '_>, size: usize) {
    bench_try_from_slice(b, Corpus::Ascii, size);
}

bench_sizes!(try_from_slice_ascii_late_non_ascii, [32, 16, 8, 1]);
fn try_from_slice_ascii_late_non_ascii(b: Bencher<'_, '_>, size: usize) {
    bench_try_from_slice(b, Corpus::AsciiLateNonAscii, size);
}

bench_sizes!(try_from_slice_multibyte, [32, 16, 4, 1]);
fn try_from_slice_multibyte(b: Bencher<'_, '_>, size: usize) {
    bench_try_from_slice(b, Corpus::Multibyte, size);
}

bench_sizes!(try_from_slice_invalid_early, [128, 32, 32, 32]);
fn try_from_slice_invalid_early(b: Bencher<'_, '_>, size: usize) {
    bench_try_from_slice(b, Corpus::InvalidEarly, size);
}

bench_sizes!(try_from_slice_invalid_late, [64, 16, 1, 1]);
fn try_from_slice_invalid_late(b: Bencher<'_, '_>, size: usize) {
    bench_try_from_slice(b, Corpus::InvalidLate, size);
}

bench_sizes!(try_from_vec_ascii, [128, 128, 32, 1]);
fn try_from_vec_ascii(b: Bencher<'_, '_>, size: usize) {
    bench_try_from_vec(b, Corpus::Ascii, size);
}

bench_sizes!(try_from_vec_ascii_late_non_ascii, [64, 64, 16, 1]);
fn try_from_vec_ascii_late_non_ascii(b: Bencher<'_, '_>, size: usize) {
    bench_try_from_vec(b, Corpus::AsciiLateNonAscii, size);
}

bench_sizes!(try_from_vec_multibyte, [64, 64, 4, 1]);
fn try_from_vec_multibyte(b: Bencher<'_, '_>, size: usize) {
    bench_try_from_vec(b, Corpus::Multibyte, size);
}

bench_sizes!(try_from_vec_invalid_early, [64, 32, 32, 8]);
fn try_from_vec_invalid_early(b: Bencher<'_, '_>, size: usize) {
    bench_try_from_vec(b, Corpus::InvalidEarly, size);
}

bench_sizes!(try_from_vec_invalid_late, [32, 8, 1, 1]);
fn try_from_vec_invalid_late(b: Bencher<'_, '_>, size: usize) {
    bench_try_from_vec(b, Corpus::InvalidLate, size);
}

bench_sizes!(try_from_bytes_mut_ascii, [128, 64, 32, 1]);
fn try_from_bytes_mut_ascii(b: Bencher<'_, '_>, size: usize) {
    bench_try_from_bytes_mut(b, Corpus::Ascii, size);
}

bench_sizes!(try_from_bytes_mut_ascii_late_non_ascii, [128, 32, 16, 1]);
fn try_from_bytes_mut_ascii_late_non_ascii(b: Bencher<'_, '_>, size: usize) {
    bench_try_from_bytes_mut(b, Corpus::AsciiLateNonAscii, size);
}

bench_sizes!(try_from_bytes_mut_multibyte, [64, 32, 4, 1]);
fn try_from_bytes_mut_multibyte(b: Bencher<'_, '_>, size: usize) {
    bench_try_from_bytes_mut(b, Corpus::Multibyte, size);
}

bench_sizes!(try_from_bytes_mut_invalid_early, [64, 32, 16, 8]);
fn try_from_bytes_mut_invalid_early(b: Bencher<'_, '_>, size: usize) {
    bench_try_from_bytes_mut(b, Corpus::InvalidEarly, size);
}

bench_sizes!(try_from_bytes_mut_invalid_late, [32, 8, 1, 1]);
fn try_from_bytes_mut_invalid_late(b: Bencher<'_, '_>, size: usize) {
    bench_try_from_bytes_mut(b, Corpus::InvalidLate, size);
}

fn main() {
    divan::main();
}
