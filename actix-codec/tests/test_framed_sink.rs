use std::{
    collections::VecDeque,
    io::{self, Write},
    pin::Pin,
    task::{
        Context,
        Poll::{self, Pending, Ready},
    },
};

use actix_codec::*;
use bytes::{Buf as _, BufMut as _, BytesMut};
use futures_sink::Sink;
use tokio_test::{assert_ready, task};

macro_rules! bilateral {
    ($($x:expr,)*) => {{
        let mut v = VecDeque::new();
        v.extend(vec![$($x),*]);
        Bilateral { calls: v }
    }};
}

macro_rules! assert_ready {
    ($e:expr) => {{
        use core::task::Poll::*;
        match $e {
            Ready(v) => v,
            Pending => panic!("pending"),
        }
    }};
    ($e:expr, $($msg:tt),+) => {{
        use core::task::Poll::*;
        match $e {
            Ready(v) => v,
            Pending => {
                let msg = format_args!($($msg),+);
                panic!("pending; {}", msg)
            }
        }
    }};
}

#[derive(Debug)]
pub struct Bilateral {
    pub calls: VecDeque<io::Result<Vec<u8>>>,
}

impl Write for Bilateral {
    fn write(&mut self, src: &[u8]) -> io::Result<usize> {
        match self.calls.pop_front() {
            Some(Ok(data)) => {
                assert!(src.len() >= data.len());
                assert_eq!(&data[..], &src[..data.len()]);
                Ok(data.len())
            }
            Some(Err(err)) => Err(err),
            None => panic!("unexpected write; {:?}", src),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl AsyncWrite for Bilateral {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        match Pin::get_mut(self).write(buf) {
            Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => Pending,
            other => Ready(other),
        }
    }
    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        match Pin::get_mut(self).flush() {
            Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => Pending,
            other => Ready(other),
        }
    }
    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        unimplemented!()
    }
}

impl AsyncRead for Bilateral {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        use io::ErrorKind::WouldBlock;

        match self.calls.pop_front() {
            Some(Ok(data)) => {
                debug_assert!(buf.remaining() >= data.len());
                buf.put_slice(&data);
                Ready(Ok(()))
            }
            Some(Err(ref err)) if err.kind() == WouldBlock => Pending,
            Some(Err(err)) => Ready(Err(err)),
            None => Ready(Ok(())),
        }
    }
}

pub struct U32;

impl Encoder<u32> for U32 {
    type Error = io::Error;

    fn encode(&mut self, item: u32, dst: &mut BytesMut) -> io::Result<()> {
        // Reserve space
        dst.reserve(4);
        dst.put_u32(item);
        Ok(())
    }
}

impl Decoder for U32 {
    type Item = u32;
    type Error = io::Error;

    fn decode(&mut self, buf: &mut BytesMut) -> io::Result<Option<u32>> {
        if buf.len() < 4 {
            return Ok(None);
        }

        let n = buf.split_to(4).get_u32();
        Ok(Some(n))
    }
}

#[test]
fn test_write_hits_highwater_mark() {
    // see here for what this test is based on:
    // https://github.com/tokio-rs/tokio/blob/75c07770bfbfea4e5fd914af819c741ed9c3fc36/tokio-util/tests/framed_write.rs#L69

    const ITER: usize = 2 * 1024;

    let mut bi = bilateral! {
        Err(io::Error::new(io::ErrorKind::WouldBlock, "not ready")),
        Ok(b"".to_vec()),
    };

    for i in 0..=ITER {
        let mut b = BytesMut::with_capacity(4);
        b.put_u32(i as u32);

        // Append to the end
        match bi.calls.back_mut().unwrap() {
            Ok(ref mut data) => {
                // Write in 2kb chunks
                if data.len() < ITER {
                    data.extend_from_slice(&b[..]);
                    continue;
                } // else fall through and create a new buffer
            }
            _ => unreachable!(),
        }

        // Push a new new chunk
        bi.calls.push_back(Ok(b[..].to_vec()));
    }

    assert_eq!(bi.calls.len(), 6);
    let mut framed = Framed::new(bi, U32);
    // Send 8KB. This fills up FramedWrite2 buffer
    let mut task = task::spawn(());
    task.enter(|cx, _| {
        // Send 8KB. This fills up Framed buffer
        for i in 0..ITER {
            {
                #[allow(unused_mut)]
                let mut framed = Pin::new(&mut framed);
                assert!(assert_ready!(framed.poll_ready(cx)).is_ok());
            }

            #[allow(unused_mut)]
            let mut framed = Pin::new(&mut framed);
            // write the buffer
            assert!(framed.start_send(i as u32).is_ok());
        }

        {
            #[allow(unused_mut)]
            let mut framed = Pin::new(&mut framed);

            // Now we poll_ready which forces a flush. The bilateral pops the front message
            // and decides to block.
            assert!(framed.poll_ready(cx).is_pending());
        }

        {
            #[allow(unused_mut)]
            let mut framed = Pin::new(&mut framed);
            // We poll again, forcing another flush, which this time succeeds
            // The whole 8KB buffer is flushed
            assert!(assert_ready!(framed.poll_ready(cx)).is_ok());
        }

        {
            #[allow(unused_mut)]
            let mut framed = Pin::new(&mut framed);
            // Send more data. This matches the final message expected by the bilateral
            assert!(framed.start_send(ITER as u32).is_ok());
        }

        {
            #[allow(unused_mut)]
            let mut framed = Pin::new(&mut framed);
            // Flush the rest of the buffer
            assert!(assert_ready!(framed.poll_flush(cx)).is_ok());
        }

        // Ensure the mock is empty
        assert_eq!(0, Pin::new(&framed).get_ref().io_ref().calls.len());
    });
}
