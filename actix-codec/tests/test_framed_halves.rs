#![allow(missing_docs)]

use std::{io, pin::Pin};

use actix_codec::{Decoder, Encoder, Framed, LinesCodec};
use bytes::BytesMut;
use futures_core::Stream;
use futures_sink::Sink;
use tokio_test::{assert_ready, task};

struct ReadCodec(LinesCodec);

impl Decoder for ReadCodec {
    type Item = String;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> io::Result<Option<String>> {
        self.0.decode(src)
    }
}

struct WriteCodec;

impl Encoder<&[u8]> for WriteCodec {
    type Error = io::Error;

    fn encode(&mut self, item: &[u8], dst: &mut BytesMut) -> io::Result<()> {
        dst.extend_from_slice(item);
        Ok(())
    }
}

#[test]
fn read_half_with_decoder_only() {
    let io = tokio_test::io::Builder::new().read(b"hello\n").build();
    let (read, _write) = tokio::io::split(io);
    let mut framed = Framed::new(read, ReadCodec(LinesCodec::default()));

    task::spawn(()).enter(|cx, _| {
        let item = assert_ready!(Pin::new(&mut framed).poll_next(cx));
        assert_eq!(item.unwrap().unwrap(), "hello");
        assert!(assert_ready!(Pin::new(&mut framed).poll_next(cx)).is_none());
    });
}

#[test]
fn write_half_with_encoder_only() {
    let io = tokio_test::io::Builder::new().write(b"hello\n").build();
    let (_read, write) = tokio::io::split(io);
    let mut framed = Framed::new(write, WriteCodec);

    task::spawn(()).enter(|cx, _| {
        let mut framed = Pin::new(&mut framed);
        assert_ready!(framed.as_mut().poll_ready(cx)).unwrap();
        framed.as_mut().start_send(&b"hello\n"[..]).unwrap();
        assert_ready!(framed.as_mut().poll_flush(cx)).unwrap();
        assert_ready!(framed.poll_close(cx)).unwrap();
    });
}
