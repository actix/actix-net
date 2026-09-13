//! Buffer initialization and backpressure coverage.
use std::{
    future::poll_fn,
    pin::Pin,
    task::{Context, Poll, Waker},
    time::Duration,
};

use actix_codec::{AsyncRead, AsyncWrite, Framed, LinesCodec};
use futures_sink::Sink;
use tokio::io::AsyncWriteExt as _;

async fn send<T: AsyncWrite + Unpin>(framed: &mut Framed<T, LinesCodec>, line: &str) {
    poll_fn(|cx| <Framed<T, LinesCodec> as Sink<&str>>::poll_ready(Pin::new(&mut *framed), cx))
        .await
        .unwrap();
    Pin::new(&mut *framed).write(line).unwrap();
    poll_fn(|cx| Pin::new(&mut *framed).flush::<&str>(cx))
        .await
        .unwrap();
}

async fn receive<T: AsyncRead + Unpin>(framed: &mut Framed<T, LinesCodec>) -> String {
    poll_fn(|cx| Pin::new(&mut *framed).next_item(cx))
        .await
        .unwrap()
        .unwrap()
}

#[test]
fn unused_transport_has_no_buffer_storage() {
    let parts = Framed::new(tokio::io::empty(), LinesCodec::new()).into_parts();
    assert_eq!(parts.read_buf.capacity(), 0);
    assert_eq!(parts.write_buf.capacity(), 0);
}

#[test]
fn first_read_does_not_allocate_write_buffer() {
    let mut framed = Framed::new(&b"hello\n"[..], LinesCodec::new());
    let mut cx = Context::from_waker(Waker::noop());
    assert!(
        matches!(Pin::new(&mut framed).next_item(&mut cx), Poll::Ready(Some(Ok(line))) if line == "hello")
    );
    let parts = framed.into_parts();
    assert!(parts.read_buf.capacity() > 0);
    assert_eq!(parts.write_buf.capacity(), 0);
}

#[test]
fn first_write_does_not_allocate_read_buffer() {
    let mut framed = Framed::new(tokio::io::sink(), LinesCodec::new());
    Pin::new(&mut framed).write("hello").unwrap();
    let parts = framed.into_parts();
    assert_eq!(parts.read_buf.capacity(), 0);
    assert_eq!(&parts.write_buf[..], b"hello\n");
    assert!(parts.write_buf.capacity() >= 8192);
}

#[tokio::test]
async fn bidirectional_roundtrips_with_small_io_buffers() {
    for capacity in [1, 31, 8192] {
        for size in [0, 1, 63, 1024, 8192, 65536] {
            let payload = "x".repeat(size);
            let (client, server) = tokio::io::duplex(capacity);
            let mut client = Framed::new(client, LinesCodec::new());
            let mut server = Framed::new(server, LinesCodec::new());
            tokio::time::timeout(Duration::from_secs(30), async {
                tokio::join!(
                    async {
                        for _ in 0..4 {
                            send(&mut client, &payload).await;
                            assert_eq!(receive(&mut client).await, payload);
                        }
                        client.io_mut().shutdown().await.unwrap();
                    },
                    async {
                        for _ in 0..4 {
                            let line = receive(&mut server).await;
                            assert_eq!(line, payload);
                            send(&mut server, &line).await;
                        }
                        assert!(poll_fn(|cx| Pin::new(&mut server).next_item(cx))
                            .await
                            .is_none());
                    }
                );
            })
            .await
            .unwrap();
        }
    }
}

#[tokio::test]
async fn pending_read_wakes_when_input_arrives() {
    let (reader, mut writer) = tokio::io::duplex(1);
    let mut framed = Framed::new(reader, LinesCodec::new());
    let mut task = tokio_test::task::spawn(poll_fn(|cx| Pin::new(&mut framed).next_item(cx)));
    assert!(task.poll().is_pending());
    writer.write_all(b"x").await.unwrap();
    assert!(task.is_woken());
    assert!(task.poll().is_pending());
    writer.write_all(b"\n").await.unwrap();
    assert!(task.is_woken());
    assert!(matches!(task.poll(), Poll::Ready(Some(Ok(line))) if line == "x"));
}
