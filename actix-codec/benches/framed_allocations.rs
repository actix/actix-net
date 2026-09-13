//! Allocation checks for unused and one-sided transports.
use std::{
    pin::Pin,
    task::{Context, Poll, Waker},
};

use actix_codec::{Framed, LinesCodec};

#[global_allocator]
static ALLOC: divan::AllocProfiler = divan::AllocProfiler::system();

fn main() {
    divan::main();
}

#[divan::bench]
fn idle() -> Framed<tokio::io::Empty, LinesCodec> {
    Framed::new(tokio::io::empty(), LinesCodec::new())
}

#[divan::bench]
fn first_read() -> Framed<&'static [u8], LinesCodec> {
    let mut framed = Framed::new(&b"hello\n"[..], LinesCodec::new());
    let mut cx = Context::from_waker(Waker::noop());
    assert!(
        matches!(Pin::new(&mut framed).next_item(&mut cx), Poll::Ready(Some(Ok(line))) if line == "hello")
    );
    framed
}

#[divan::bench]
fn first_write() -> Framed<tokio::io::Sink, LinesCodec> {
    let mut framed = Framed::new(tokio::io::sink(), LinesCodec::new());
    Pin::new(&mut framed).write("hello").unwrap();
    framed
}
