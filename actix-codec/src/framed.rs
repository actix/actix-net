#![allow(deprecated)]

use std::fmt;
use std::io::{self};
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::BytesMut;
use futures::{ready, Sink, Stream};
use pin_project::pin_project;
use tokio_util::codec::{Decoder, Encoder};
use tokio::io::{AsyncRead, AsyncWrite};

const LW: usize = 1024;
const HW: usize = 8 * 1024;
const INITIAL_CAPACITY: usize = 8 * 1024;

/// A unified `Stream` and `Sink` interface to an underlying I/O object, using
/// the `Encoder` and `Decoder` traits to encode and decode frames.
///
/// You can create a `Framed` instance by using the `AsyncRead::framed` adapter.
#[pin_project]
pub struct Framed<T, U> {
    io: T,
    codec: U,
    eof: bool,
    is_readable: bool,
    read_buf: BytesMut,
    write_buf: BytesMut,
    write_lw: usize,
    write_hw: usize,
}

impl<T, U> Framed<T, U>
where
    T: AsyncRead + AsyncWrite,
    U: Decoder + Encoder,
{
    /// Provides a `Stream` and `Sink` interface for reading and writing to this
    /// `Io` object, using `Decode` and `Encode` to read and write the raw data.
    ///
    /// Raw I/O objects work with byte sequences, but higher-level code usually
    /// wants to batch these into meaningful chunks, called "frames". This
    /// method layers framing on top of an I/O object, by using the `Codec`
    /// traits to handle encoding and decoding of messages frames. Note that
    /// the incoming and outgoing frame types may be distinct.
    ///
    /// This function returns a *single* object that is both `Stream` and
    /// `Sink`; grouping this into a single object is often useful for layering
    /// things like gzip or TLS, which require both read and write access to the
    /// underlying object.
    ///
    /// If you want to work more directly with the streams and sink, consider
    /// calling `split` on the `Framed` returned by this method, which will
    /// break them into separate objects, allowing them to interact more easily.
    pub fn new(io: T, codec: U) -> Framed<T, U> {
        Framed {
            io,
            codec,
            eof: false,
            is_readable: false,
            read_buf: BytesMut::with_capacity(INITIAL_CAPACITY),
            write_buf: BytesMut::with_capacity(HW),
            write_lw: LW,
            write_hw: HW,
        }
    }

    /// Same as `Framed::new()` with ability to specify write buffer low/high capacity watermarks.
    pub fn new_with_caps(io: T, codec: U, lw: usize, hw: usize) -> Framed<T, U> {
        debug_assert!((lw < hw) && hw != 0);
        Framed {
            io,
            codec,
            eof: false,
            is_readable: false,
            read_buf: BytesMut::with_capacity(INITIAL_CAPACITY),
            write_buf: BytesMut::with_capacity(hw),
            write_lw: lw,
            write_hw: hw,
        }
    }
}

impl<T, U> Framed<T, U> {
    /// Provides a `Stream` and `Sink` interface for reading and writing to this
    /// `Io` object, using `Decode` and `Encode` to read and write the raw data.
    ///
    /// Raw I/O objects work with byte sequences, but higher-level code usually
    /// wants to batch these into meaningful chunks, called "frames". This
    /// method layers framing on top of an I/O object, by using the `Codec`
    /// traits to handle encoding and decoding of messages frames. Note that
    /// the incoming and outgoing frame types may be distinct.
    ///
    /// This function returns a *single* object that is both `Stream` and
    /// `Sink`; grouping this into a single object is often useful for layering
    /// things like gzip or TLS, which require both read and write access to the
    /// underlying object.
    ///
    /// This objects takes a stream and a readbuffer and a writebuffer. These
    /// field can be obtained from an existing `Framed` with the
    /// `into_parts` method.
    ///
    /// If you want to work more directly with the streams and sink, consider
    /// calling `split` on the `Framed` returned by this method, which will
    /// break them into separate objects, allowing them to interact more easily.
    pub fn from_parts(parts: FramedParts<T, U>) -> Framed<T, U> {
        Framed {
            io: parts.io,
            codec: parts.codec,
            eof: false,
            is_readable: false,
            write_buf: parts.write_buf,
            write_lw: parts.write_buf_lw,
            write_hw: parts.write_buf_hw,
            read_buf: parts.read_buf,
        }
    }

    /// Returns a reference to the underlying codec.
    pub fn get_codec(&self) -> &U {
        &self.codec
    }

    /// Returns a mutable reference to the underlying codec.
    pub fn get_codec_mut(&mut self) -> &mut U {
        &mut self.codec
    }

    /// Returns a reference to the underlying I/O stream wrapped by
    /// `Frame`.
    ///
    /// Note that care should be taken to not tamper with the underlying stream
    /// of data coming in as it may corrupt the stream of frames otherwise
    /// being worked with.
    pub fn get_ref(&self) -> &T {
        &self.io
    }

    /// Returns a mutable reference to the underlying I/O stream wrapped by
    /// `Frame`.
    ///
    /// Note that care should be taken to not tamper with the underlying stream
    /// of data coming in as it may corrupt the stream of frames otherwise
    /// being worked with.
    pub fn get_mut(&mut self) -> &mut T {
        &mut self.io
    }

    /// Check if write buffer is empty.
    pub fn is_write_buf_empty(&self) -> bool {
        self.write_buf.is_empty()
    }

    /// Check if write buffer is full.
    pub fn is_write_buf_full(&self) -> bool {
        self.write_buf.len() >= self.write_hw
    }

    /// Consumes the `Frame`, returning its underlying I/O stream.
    ///
    /// Note that care should be taken to not tamper with the underlying stream
    /// of data coming in as it may corrupt the stream of frames otherwise
    /// being worked with.
    pub fn into_inner(self) -> T {
        self.io
    }

    /// Consume the `Frame`, returning `Frame` with different codec.
    pub fn into_framed<U2>(self, codec: U2) -> Framed<T, U2> {
        Framed {
            io: self.io,
            codec,
            eof: self.eof,
            is_readable: self.is_readable,
            read_buf: self.read_buf,
            write_buf: self.write_buf,
            write_lw: self.write_lw,
            write_hw: self.write_hw,
        }
    }

    /// Consume the `Frame`, returning `Frame` with different io.
    pub fn map_io<F, T2>(self, f: F) -> Framed<T2, U>
    where
        F: Fn(T) -> T2,
    {
        Framed {
            io: f(self.io),
            codec: self.codec,
            eof: self.eof,
            is_readable: self.is_readable,
            read_buf: self.read_buf,
            write_buf: self.write_buf,
            write_lw: self.write_lw,
            write_hw: self.write_hw,
        }
    }

    /// Consume the `Frame`, returning `Frame` with different codec.
    pub fn map_codec<F, U2>(self, f: F) -> Framed<T, U2>
    where
        F: Fn(U) -> U2,
    {
        Framed {
            io: self.io,
            codec: f(self.codec),
            eof: self.eof,
            is_readable: self.is_readable,
            read_buf: self.read_buf,
            write_buf: self.write_buf,
            write_lw: self.write_lw,
            write_hw: self.write_hw,
        }
    }

    /// Consumes the `Frame`, returning its underlying I/O stream, the buffer
    /// with unprocessed data, and the codec.
    ///
    /// Note that care should be taken to not tamper with the underlying stream
    /// of data coming in as it may corrupt the stream of frames otherwise
    /// being worked with.
    pub fn into_parts(self) -> FramedParts<T, U> {
        FramedParts {
            io: self.io,
            codec: self.codec,
            read_buf: self.read_buf,
            write_buf: self.write_buf,
            write_buf_lw: self.write_lw,
            write_buf_hw: self.write_hw,
            _priv: (),
        }
    }
}

impl<T, U> Framed<T, U> {
    /// Serialize item and Write to the inner buffer
    pub fn write(&mut self, item: <U as Encoder>::Item) -> Result<(), <U as Encoder>::Error>
    where
        T: AsyncWrite,
        U: Encoder,
    {
        let len = self.write_buf.len();
        if len < self.write_lw {
            self.write_buf.reserve(self.write_hw - len)
        }
        self.codec.encode(item, &mut self.write_buf)?;
        Ok(())
    }

    pub fn is_ready(&self) -> bool {
        let len = self.write_buf.len();
        len < self.write_hw
    }

    pub fn next_item(&mut self, cx: &mut Context) -> Poll<Option<Result<U::Item, U::Error>>>
    where
        T: AsyncRead,
        U: Decoder,
    {
        loop {
            // Repeatedly call `decode` or `decode_eof` as long as it is
            // "readable". Readable is defined as not having returned `None`. If
            // the upstream has returned EOF, and the decoder is no longer
            // readable, it can be assumed that the decoder will never become
            // readable again, at which point the stream is terminated.

            if self.is_readable {
                if self.eof {
                    match self.codec.decode_eof(&mut self.read_buf) {
                        Ok(Some(frame)) => return Poll::Ready(Some(Ok(frame))),
                        Ok(None) => return Poll::Ready(None),
                        Err(e) => return Poll::Ready(Some(Err(e))),
                    }
                }

                log::trace!("attempting to decode a frame");

                match self.codec.decode(&mut self.read_buf) {
                    Ok(Some(frame)) => {
                        log::trace!("frame decoded from buffer");
                        return Poll::Ready(Some(Ok(frame)));
                    }
                    Err(e) => return Poll::Ready(Some(Err(e))),
                    _ => {
                        // Need more data
                    }
                }

                self.is_readable = false;
            }

            assert!(!self.eof);

            // Otherwise, try to read more data and try again. Make sure we've
            // got room for at least one byte to read to ensure that we don't
            // get a spurious 0 that looks like EOF
            self.read_buf.reserve(1);
            let cnt = unsafe {
                match Pin::new_unchecked(&mut self.io).poll_read_buf(cx, &mut self.read_buf) {
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(Err(e)) => return Poll::Ready(Some(Err(e.into()))),
                    Poll::Ready(Ok(cnt)) => cnt,
                }
            };

            if cnt == 0 {
                self.eof = true;
            }
            self.is_readable = true;
        }
    }

    pub fn flush(&mut self, cx: &mut Context) -> Poll<Result<(), U::Error>>
    where
        T: AsyncWrite,
        U: Encoder,
    {
        log::trace!("flushing framed transport");

        while !self.write_buf.is_empty() {
            log::trace!("writing; remaining={}", self.write_buf.len());

            let n = ready!(
                unsafe { Pin::new_unchecked(&mut self.io) }.poll_write(cx, &self.write_buf)
            )?;

            if n == 0 {
                return Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "failed to \
                     write frame to transport",
                )
                .into()));
            }

            // TODO: Add a way to `bytes` to do this w/o returning the drained
            // data.
            let _ = self.write_buf.split_to(n);
        }

        // Try flushing the underlying IO
        ready!(unsafe { Pin::new_unchecked(&mut self.io) }.poll_flush(cx))?;

        log::trace!("framed transport flushed");
        Poll::Ready(Ok(()))
    }

    pub fn close(&mut self, cx: &mut Context) -> Poll<Result<(), U::Error>>
    where
        T: AsyncWrite,
        U: Encoder,
    {
        ready!(unsafe { Pin::new_unchecked(&mut self.io) }.poll_flush(cx))?;
        ready!(unsafe { Pin::new_unchecked(&mut self.io) }.poll_shutdown(cx))?;

        Poll::Ready(Ok(()))
    }
}

impl<T, U> Stream for Framed<T, U>
where
    T: AsyncRead,
    U: Decoder,
{
    type Item = Result<U::Item, U::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        self.next_item(cx)
    }
}

impl<T, U> Sink<U::Item> for Framed<T, U>
where
    T: AsyncWrite,
    U: Encoder,
    U::Error: From<io::Error>,
{
    type Error = U::Error;

    fn poll_ready(self: Pin<&mut Self>, _: &mut Context) -> Poll<Result<(), Self::Error>> {
        if self.is_ready() {
            Poll::Ready(Ok(()))
        } else {
            Poll::Pending
        }
    }

    fn start_send(
        mut self: Pin<&mut Self>,
        item: <U as Encoder>::Item,
    ) -> Result<(), Self::Error> {
        self.write(item)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        self.flush(cx)
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        self.close(cx)
    }
}

impl<T, U> fmt::Debug for Framed<T, U>
where
    T: fmt::Debug,
    U: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("Framed")
            .field("io", &self.io)
            .field("codec", &self.codec)
            .finish()
    }
}

/// `FramedParts` contains an export of the data of a Framed transport.
/// It can be used to construct a new `Framed` with a different codec.
/// It contains all current buffers and the inner transport.
#[derive(Debug)]
pub struct FramedParts<T, U> {
    /// The inner transport used to read bytes to and write bytes to
    pub io: T,

    /// The codec
    pub codec: U,

    /// The buffer with read but unprocessed data.
    pub read_buf: BytesMut,

    /// A buffer with unprocessed data which are not written yet.
    pub write_buf: BytesMut,

    /// A buffer low watermark capacity
    pub write_buf_lw: usize,

    /// A buffer high watermark capacity
    pub write_buf_hw: usize,

    /// This private field allows us to add additional fields in the future in a
    /// backwards compatible way.
    _priv: (),
}

impl<T, U> FramedParts<T, U> {
    /// Create a new, default, `FramedParts`
    pub fn new(io: T, codec: U) -> FramedParts<T, U> {
        FramedParts {
            io,
            codec,
            read_buf: BytesMut::new(),
            write_buf: BytesMut::new(),
            write_buf_lw: LW,
            write_buf_hw: HW,
            _priv: (),
        }
    }

    /// Create a new `FramedParts` with read buffer
    pub fn with_read_buf(io: T, codec: U, read_buf: BytesMut) -> FramedParts<T, U> {
        FramedParts {
            io,
            codec,
            read_buf,
            write_buf: BytesMut::new(),
            write_buf_lw: LW,
            write_buf_hw: HW,
            _priv: (),
        }
    }
}
