//! Utilities for encoding and decoding frames.
//!
//! Contains adapters to go from streams of bytes, [`AsyncRead`] and
//! [`AsyncWrite`], to framed streams implementing [`Sink`] and [`Stream`].
//! Framed streams are also known as `transports`.
//!
//! [`AsyncRead`]: AsyncRead
//! [`AsyncWrite`]: AsyncWrite
//! [`Sink`]: futures_sink::Sink
//! [`Stream`]: futures_core::Stream

#![deny(rust_2018_idioms)]
#![warn(missing_docs)]

mod bcodec;
mod framed;

pub use self::bcodec::BytesCodec;
pub use self::framed::{Framed, FramedParts};

pub use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
pub use tokio_util::codec::{Decoder, Encoder};

// FIXME: Remove this
/// temporary mod
pub mod util {
    use super::*;

    use std::pin::Pin;
    use std::task::{Context, Poll};

    /// temporary poll_read_buf function until it lands in tokio_util
    pub fn poll_read_buf<T: AsyncRead, B: bytes::BufMut>(
        io: Pin<&mut T>,
        cx: &mut Context<'_>,
        buf: &mut B,
    ) -> Poll<std::io::Result<usize>> {
        if !buf.has_remaining_mut() {
            return Poll::Ready(Ok(0));
        }

        let n = {
            let dst = buf.bytes_mut();
            let dst = unsafe { &mut *(dst as *mut _ as *mut [std::mem::MaybeUninit<u8>]) };
            let mut buf = ReadBuf::uninit(dst);
            let ptr = buf.filled().as_ptr();
            futures_core::ready!(io.poll_read(cx, &mut buf)?);

            // Ensure the pointer does not change from under us
            assert_eq!(ptr, buf.filled().as_ptr());
            buf.filled().len()
        };

        // Safety: This is guaranteed to be the number of initialized (and read)
        // bytes due to the invariants provided by `ReadBuf::filled`.
        unsafe {
            buf.advance_mut(n);
        }

        Poll::Ready(Ok(n))
    }
}
