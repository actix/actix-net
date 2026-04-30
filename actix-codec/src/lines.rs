use std::io;

use bytes::{Buf, BufMut, Bytes, BytesMut};
use memchr::memchr;

use super::{Decoder, Encoder};

/// Lines codec. Reads/writes line delimited strings.
///
/// Will split input up by LF or CRLF delimiters. Carriage return characters at the end of lines are
/// not preserved.
///
/// # Security
///
/// When used with untrusted input, it is recommended to set a maximum line length with
/// [`LinesCodec::new_with_max_length`]. Without a length limit, the internal read buffer can grow
/// without bound if a peer sends an unbounded amount of data without a `\n`, potentially leading
/// to memory exhaustion (DoS).
///
/// # Direct `Decoder` Use
///
/// `LinesCodec` caches the index after the last byte it searched when [`Decoder::decode`] returns
/// `Ok(None)`. Callers that invoke `decode` directly must pass the same [`BytesMut`] with newly
/// read bytes appended. If the buffer is replaced or bytes before the cached offset are changed,
/// create a new `LinesCodec` before decoding the replacement buffer.
#[derive(Debug, Copy, Clone)]
#[non_exhaustive]
pub struct LinesCodec {
    max_length: usize,
    // Next byte index to examine for `\n` after a previous incomplete decode.
    next_index: usize,
}

impl LinesCodec {
    /// Creates a new `LinesCodec` with no maximum line length.
    ///
    /// Consider using [`LinesCodec::new_with_max_length`] when working with untrusted input.
    pub const fn new() -> Self {
        Self {
            max_length: usize::MAX,
            next_index: 0,
        }
    }

    /// Creates a new `LinesCodec` with a maximum line length, in bytes.
    ///
    /// The limit applies to the bytes before the line delimiter (`\n`). If present, a trailing
    /// carriage return (`\r`) in `\r\n` sequences is not counted towards the limit.
    ///
    /// Using a length limit is recommended when working with untrusted input to avoid unbounded
    /// buffering.
    pub const fn new_with_max_length(max_length: usize) -> Self {
        Self {
            max_length,
            next_index: 0,
        }
    }

    /// Returns the maximum permitted line length, in bytes.
    pub const fn max_length(&self) -> usize {
        self.max_length
    }
}

impl Default for LinesCodec {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: AsRef<str>> Encoder<T> for LinesCodec {
    type Error = io::Error;

    #[inline]
    fn encode(&mut self, item: T, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let item = item.as_ref();
        dst.reserve(item.len() + 1);
        dst.put_slice(item.as_bytes());
        dst.put_u8(b'\n');
        Ok(())
    }
}

impl Decoder for LinesCodec {
    type Item = String;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.is_empty() {
            self.next_index = 0;
            return Ok(None);
        }

        let start = self.next_index.min(src.len());
        debug_assert!(
            memchr(b'\n', &src[..start]).is_none(),
            "LinesCodec buffer changed before cached search offset"
        );

        let len = match memchr(b'\n', &src[start..]) {
            Some(n) => start + n,
            None => {
                let max = self.max_length;
                if max != usize::MAX {
                    // No delimiter yet; if current buffered data already exceeds the maximum line
                    // length, abort to avoid unbounded memory growth.
                    let max_cr = max.saturating_add(1);

                    if src.len() > max && !(src.len() == max_cr && src.last() == Some(&b'\r')) {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "max line length exceeded",
                        ));
                    }
                }

                self.next_index = src.len();
                return Ok(None);
            }
        };

        // Reject overly long lines before splitting/advancing buffers.
        let max = self.max_length;
        if max != usize::MAX {
            let max_cr = max.saturating_add(1);

            if len > max && !(len == max_cr && src.get(len - 1) == Some(&b'\r')) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "max line length exceeded",
                ));
            }
        }

        self.next_index = 0;

        // split up to new line char
        let mut buf = src.split_to(len);
        debug_assert_eq!(len, buf.len());

        // remove new line char from source
        src.advance(1);

        match buf.last() {
            // remove carriage returns at the end of buf
            Some(b'\r') => buf.truncate(len - 1),

            // line is empty
            None => return Ok(Some(String::new())),

            _ => {}
        }

        try_into_utf8(buf.freeze())
    }

    fn decode_eof(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        match self.decode(src)? {
            Some(frame) => Ok(Some(frame)),
            None if src.is_empty() => Ok(None),
            None => {
                self.next_index = 0;

                let buf = match src.last() {
                    // if last line ends in a CR then take everything up to it
                    Some(b'\r') => src.split_to(src.len() - 1),

                    // take all bytes from source
                    _ => src.split(),
                };

                if buf.is_empty() {
                    return Ok(None);
                }

                try_into_utf8(buf.freeze())
            }
        }
    }
}

// Attempts to convert bytes into a `String`.
fn try_into_utf8(buf: Bytes) -> io::Result<Option<String>> {
    String::from_utf8(buf.to_vec())
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
        .map(Some)
}

#[cfg(test)]
mod tests {
    use bytes::BufMut as _;

    use super::*;

    #[test]
    fn lines_decoder() {
        let mut codec = LinesCodec::default();
        let mut buf = BytesMut::from("\nline 1\nline 2\r\nline 3\n\r\n\r");

        assert_eq!("", codec.decode(&mut buf).unwrap().unwrap());
        assert_eq!("line 1", codec.decode(&mut buf).unwrap().unwrap());
        assert_eq!("line 2", codec.decode(&mut buf).unwrap().unwrap());
        assert_eq!("line 3", codec.decode(&mut buf).unwrap().unwrap());
        assert_eq!("", codec.decode(&mut buf).unwrap().unwrap());
        assert!(codec.decode(&mut buf).unwrap().is_none());
        assert!(codec.decode_eof(&mut buf).unwrap().is_none());

        buf.put_slice(b"k");
        assert!(codec.decode(&mut buf).unwrap().is_none());
        assert_eq!("\rk", codec.decode_eof(&mut buf).unwrap().unwrap());

        assert!(codec.decode(&mut buf).unwrap().is_none());
        assert!(codec.decode_eof(&mut buf).unwrap().is_none());
    }

    #[test]
    fn lines_encoder() {
        let mut codec = LinesCodec::default();

        let mut buf = BytesMut::new();

        codec.encode("", &mut buf).unwrap();
        assert_eq!(&buf[..], b"\n");

        codec.encode("test", &mut buf).unwrap();
        assert_eq!(&buf[..], b"\ntest\n");

        codec.encode("a\nb", &mut buf).unwrap();
        assert_eq!(&buf[..], b"\ntest\na\nb\n");
    }

    #[test]
    fn lines_encoder_no_overflow() {
        let mut codec = LinesCodec::default();

        let mut buf = BytesMut::new();
        codec.encode("1234567", &mut buf).unwrap();
        assert_eq!(&buf[..], b"1234567\n");

        let mut buf = BytesMut::new();
        codec.encode("12345678", &mut buf).unwrap();
        assert_eq!(&buf[..], b"12345678\n");

        let mut buf = BytesMut::new();
        codec.encode("123456789111213", &mut buf).unwrap();
        assert_eq!(&buf[..], b"123456789111213\n");

        let mut buf = BytesMut::new();
        codec.encode("1234567891112131", &mut buf).unwrap();
        assert_eq!(&buf[..], b"1234567891112131\n");
    }

    #[test]
    fn lines_decoder_errors_on_overlong_line_without_delimiter() {
        let mut codec = LinesCodec::new_with_max_length(4);
        let mut buf = BytesMut::from(&b"aaaaa"[..]);

        let err = codec.decode(&mut buf).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn lines_decoder_resumes_from_previous_search() {
        let mut codec = LinesCodec::default();
        let mut buf = BytesMut::from(&b"partial"[..]);

        assert!(codec.decode(&mut buf).unwrap().is_none());

        buf.put_slice(b" line\n");
        assert_eq!("partial line", codec.decode(&mut buf).unwrap().unwrap());
        assert!(codec.decode(&mut buf).unwrap().is_none());
    }

    #[test]
    fn lines_decoder_resumes_across_multiple_partial_chunks() {
        let mut codec = LinesCodec::default();
        let mut buf = BytesMut::new();

        buf.put_slice(b"partial");
        assert!(codec.decode(&mut buf).unwrap().is_none());

        buf.put_slice(b" line");
        assert!(codec.decode(&mut buf).unwrap().is_none());

        buf.put_slice(b" across chunks");
        assert!(codec.decode(&mut buf).unwrap().is_none());

        buf.put_slice(b"\n");
        assert_eq!(
            "partial line across chunks",
            codec.decode(&mut buf).unwrap().unwrap()
        );
    }

    #[test]
    fn lines_decoder_resumes_with_max_length() {
        let mut codec = LinesCodec::new_with_max_length(18);
        let mut buf = BytesMut::new();

        buf.put_slice(b"partial");
        assert!(codec.decode(&mut buf).unwrap().is_none());

        buf.put_slice(b" line");
        assert!(codec.decode(&mut buf).unwrap().is_none());

        buf.put_slice(b" ok\n");
        assert_eq!("partial line ok", codec.decode(&mut buf).unwrap().unwrap());
    }

    #[test]
    fn lines_decoder_errors_on_overlong_partial_line() {
        let mut codec = LinesCodec::new_with_max_length(4);
        let mut buf = BytesMut::from(&b"aa"[..]);

        assert!(codec.decode(&mut buf).unwrap().is_none());

        buf.put_slice(b"aaa");
        let err = codec.decode(&mut buf).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn lines_decoder_resets_search_after_decode_eof() {
        let mut codec = LinesCodec::default();
        let mut buf = BytesMut::from(&b"partial"[..]);

        assert!(codec.decode(&mut buf).unwrap().is_none());
        assert_eq!("partial", codec.decode_eof(&mut buf).unwrap().unwrap());

        buf.put_slice(b"next\n");
        assert_eq!("next", codec.decode(&mut buf).unwrap().unwrap());
    }
}
