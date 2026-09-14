#![allow(missing_docs)]

use std::{
    collections::VecDeque,
    io,
    pin::Pin,
    task::{Context, Poll},
};

use actix_codec::{AsyncWrite, BytesCodec, Encoder, Framed, FramedParts, LinesCodec};
use bytes::{Bytes, BytesMut};
use futures_core::Stream;
use futures_sink::Sink;
use tokio_test::{assert_pending, assert_ready, task};

#[test]
fn accessors_modify_the_underlying_io_and_codec() {
    let mut framed = Framed::new(1, LinesCodec::new());

    assert_eq!(*framed.io_ref(), 1);

    *framed.io_mut() = 2;
    *Pin::new(&mut framed).io_pin() = 3;
    *framed.codec_mut() = LinesCodec::new_with_max_length(7);

    assert_eq!(*framed.io_ref(), 3);
    assert_eq!(framed.codec_ref().max_length(), 7);
    assert!(framed.is_read_buf_empty());
    assert!(framed.is_write_buf_empty());
    assert!(!framed.is_write_buf_full());
    assert!(framed.is_write_ready());
}

#[test]
fn parts_and_maps_preserve_buffers_and_transform_values() {
    let mut parts = FramedParts::with_read_buf(
        1,
        LinesCodec::new_with_max_length(7),
        BytesMut::from("read"),
    );
    parts.write_buf.extend_from_slice(b"write");

    let framed = Framed::from_parts(parts)
        .into_map_io(|io| io + 1)
        .into_map_codec(|codec| LinesCodec::new_with_max_length(codec.max_length() + 1));

    assert_eq!(framed.codec_ref().max_length(), 8);

    let parts = framed.replace_codec(BytesCodec).into_parts();

    assert_eq!(parts.io, 2);
    assert_eq!(parts.read_buf, "read");
    assert_eq!(parts.write_buf, "write");
}

#[test]
fn parts_with_read_buffer_append_input_before_decoding() {
    let io = tokio_test::io::Builder::new().read(b"fix\n").build();
    let parts = FramedParts::with_read_buf(io, LinesCodec::new(), BytesMut::from("pre"));
    let mut framed = Framed::from_parts(parts);

    task::spawn(()).enter(|cx, _| {
        assert_eq!(
            assert_ready!(Pin::new(&mut framed).poll_next(cx))
                .unwrap()
                .unwrap(),
            "prefix"
        );
    });
}

#[test]
fn transformations_preserve_readable_and_eof_state() {
    let io = tokio_test::io::Builder::new()
        .read(b"first\nsecond\nlast")
        .build();
    let mut framed = Framed::new(io, LinesCodec::new());

    task::spawn(()).enter(|cx, _| {
        assert_eq!(
            assert_ready!(Pin::new(&mut framed).poll_next(cx))
                .unwrap()
                .unwrap(),
            "first"
        );
    });

    let mut framed = Framed::from_parts(framed.into_parts())
        .into_map_io(Box::new)
        .into_map_codec(|_| LinesCodec::new())
        .replace_codec(LinesCodec::new());

    task::spawn(()).enter(|cx, _| {
        assert_eq!(
            assert_ready!(Pin::new(&mut framed).poll_next(cx))
                .unwrap()
                .unwrap(),
            "second"
        );
        assert_eq!(
            assert_ready!(Pin::new(&mut framed).poll_next(cx))
                .unwrap()
                .unwrap(),
            "last"
        );
    });

    let mut framed = Framed::from_parts(framed.into_parts())
        .into_map_io(Box::new)
        .into_map_codec(|_| LinesCodec::new())
        .replace_codec(LinesCodec::new());

    task::spawn(()).enter(|cx, _| {
        assert!(assert_ready!(Pin::new(&mut framed).poll_next(cx)).is_none());
        assert!(assert_ready!(Pin::new(&mut framed).poll_next(cx)).is_none());
    });
}

#[test]
fn read_error_is_returned() {
    let io = tokio_test::io::Builder::new()
        .read_error(io::ErrorKind::ConnectionReset.into())
        .build();
    let mut framed = Framed::new(io, BytesCodec);

    task::spawn(()).enter(|cx, _| {
        assert_eq!(
            assert_ready!(Pin::new(&mut framed).poll_next(cx))
                .unwrap()
                .unwrap_err()
                .kind(),
            io::ErrorKind::ConnectionReset
        );
    });
}

#[test]
fn decode_errors_are_returned_before_and_at_eof() {
    for input in [&b"\xff\n"[..], &b"\xff"[..]] {
        let io = tokio_test::io::Builder::new().read(input).build();
        let mut framed = Framed::new(io, LinesCodec::new());

        task::spawn(()).enter(|cx, _| {
            assert_eq!(
                assert_ready!(Pin::new(&mut framed).poll_next(cx))
                    .unwrap()
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::InvalidData
            );
        });
    }
}

#[test]
fn pending_read_resumes_when_more_data_arrives() {
    let (io, mut handle) = tokio_test::io::Builder::new()
        .read(b"part")
        .build_with_handle();
    let mut framed = Framed::new(io, LinesCodec::new());
    let mut task = task::spawn(());

    task.enter(|cx, _| assert_pending!(Pin::new(&mut framed).poll_next(cx)));

    handle.read(b"ial\n");

    task.enter(|cx, _| {
        assert_eq!(
            assert_ready!(Pin::new(&mut framed).poll_next(cx))
                .unwrap()
                .unwrap(),
            "partial"
        );
    });
}

#[test]
fn read_buffer_grows_for_large_frames() {
    let line = "a".repeat(16 * 1024);
    let mut input = line.clone();
    input.push('\n');

    let io = tokio_test::io::Builder::new()
        .read(input.as_bytes())
        .build();
    let mut framed = Framed::new(io, LinesCodec::new());

    task::spawn(()).enter(|cx, _| {
        assert_eq!(
            assert_ready!(Pin::new(&mut framed).poll_next(cx))
                .unwrap()
                .unwrap(),
            line
        );
    });
}

#[derive(Default)]
struct Writer {
    writes: VecDeque<Poll<io::Result<usize>>>,
    flushes: VecDeque<Poll<io::Result<()>>>,
    shutdowns: VecDeque<Poll<io::Result<()>>>,
    written: Vec<u8>,
}

impl AsyncWrite for Writer {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let result = self.writes.pop_front().expect("unexpected write");

        if let Poll::Ready(Ok(len)) = result {
            self.written.extend_from_slice(&buf[..len]);
        }

        result
    }

    fn poll_flush(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.flushes.pop_front().expect("unexpected flush")
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.shutdowns.pop_front().expect("unexpected shutdown")
    }
}

#[test]
fn partial_writes_preserve_remaining_bytes_while_pending() {
    let io = Writer {
        writes: [Poll::Ready(Ok(2)), Poll::Pending, Poll::Ready(Ok(3))].into(),
        flushes: [Poll::Pending, Poll::Ready(Ok(()))].into(),
        ..Writer::default()
    };
    let mut framed = Framed::from_parts(FramedParts::new(io, BytesCodec));

    Pin::new(&mut framed)
        .start_send(Bytes::from_static(b"hello"))
        .unwrap();

    task::spawn(()).enter(|cx, _| {
        assert_pending!(Pin::new(&mut framed).poll_flush(cx));
        assert_eq!(framed.io_ref().written, b"he");
        assert!(!framed.is_write_buf_empty());

        assert_pending!(Pin::new(&mut framed).poll_flush(cx));
        assert!(framed.is_write_buf_empty());

        assert_ready!(Pin::new(&mut framed).poll_flush(cx)).unwrap();
        assert_eq!(framed.io_ref().written, b"hello");
    });
}

#[test]
fn write_errors_preserve_unsent_bytes() {
    for (result, expected) in [
        (Ok(0), io::ErrorKind::WriteZero),
        (
            Err(io::ErrorKind::BrokenPipe.into()),
            io::ErrorKind::BrokenPipe,
        ),
    ] {
        let io = Writer {
            writes: [Poll::Ready(result)].into(),
            ..Writer::default()
        };
        let mut framed = Framed::new(io, BytesCodec);

        Pin::new(&mut framed)
            .start_send(Bytes::from_static(b"hello"))
            .unwrap();

        task::spawn(()).enter(|cx, _| {
            assert_eq!(
                assert_ready!(Pin::new(&mut framed).poll_flush(cx))
                    .unwrap_err()
                    .kind(),
                expected
            );
        });

        assert_eq!(framed.into_parts().write_buf, "hello");
    }
}

#[test]
fn underlying_flush_error_is_returned() {
    let io = Writer {
        flushes: [Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()))].into(),
        ..Writer::default()
    };
    let mut framed = Framed::new(io, BytesCodec);

    task::spawn(()).enter(|cx, _| {
        assert_eq!(
            assert_ready!(Pin::new(&mut framed).poll_flush(cx))
                .unwrap_err()
                .kind(),
            io::ErrorKind::BrokenPipe
        );
    });
}

#[test]
fn close_waits_for_flush_and_shutdown() {
    let io = Writer {
        flushes: [Poll::Pending, Poll::Ready(Ok(())), Poll::Ready(Ok(()))].into(),
        shutdowns: [Poll::Pending, Poll::Ready(Ok(()))].into(),
        ..Writer::default()
    };
    let mut framed = Framed::new(io, BytesCodec);

    task::spawn(()).enter(|cx, _| {
        assert_pending!(Pin::new(&mut framed).poll_close(cx));
        assert_eq!(framed.io_ref().shutdowns.len(), 2);

        assert_pending!(Pin::new(&mut framed).poll_close(cx));
        assert_ready!(Pin::new(&mut framed).poll_close(cx)).unwrap();
        assert!(framed.io_ref().shutdowns.is_empty());
    });
}

#[test]
fn close_returns_flush_error_without_shutdown() {
    let io = Writer {
        flushes: [Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()))].into(),
        ..Writer::default()
    };
    let mut framed = Framed::new(io, BytesCodec);

    task::spawn(()).enter(|cx, _| {
        assert_eq!(
            assert_ready!(Pin::new(&mut framed).poll_close(cx))
                .unwrap_err()
                .kind(),
            io::ErrorKind::BrokenPipe
        );
    });
}

#[test]
fn close_returns_shutdown_error_after_flush() {
    let io = Writer {
        flushes: [Poll::Ready(Ok(()))].into(),
        shutdowns: [Poll::Ready(Err(io::ErrorKind::ConnectionReset.into()))].into(),
        ..Writer::default()
    };
    let mut framed = Framed::new(io, BytesCodec);

    task::spawn(()).enter(|cx, _| {
        assert_eq!(
            assert_ready!(Pin::new(&mut framed).poll_close(cx))
                .unwrap_err()
                .kind(),
            io::ErrorKind::ConnectionReset
        );
    });

    assert!(framed.io_ref().flushes.is_empty());
    assert!(framed.io_ref().shutdowns.is_empty());
}

struct RejectEncoder;

impl Encoder<Bytes> for RejectEncoder {
    type Error = io::Error;

    fn encode(&mut self, _: Bytes, _: &mut BytesMut) -> io::Result<()> {
        Err(io::ErrorKind::InvalidInput.into())
    }
}

#[test]
fn encode_error_is_returned_without_writing_to_io() {
    let mut framed = Framed::new(Writer::default(), RejectEncoder);

    assert_eq!(
        Pin::new(&mut framed)
            .start_send(Bytes::from_static(b"foo"))
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidInput
    );
    assert!(framed.is_write_buf_empty());
}

#[test]
fn write_buffer_reports_high_water_mark() {
    let mut framed = Framed::new(Writer::default(), BytesCodec);

    Pin::new(&mut framed)
        .start_send(Bytes::from(vec![0; 8 * 1024 - 1]))
        .unwrap();

    assert!(!framed.is_write_buf_full());
    assert!(framed.is_write_ready());

    Pin::new(&mut framed)
        .start_send(Bytes::from_static(b"x"))
        .unwrap();

    assert!(framed.is_write_buf_full());
    assert!(!framed.is_write_ready());
}
