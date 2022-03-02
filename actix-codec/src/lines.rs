use std::io;

use bytes::{Buf, BufMut, Bytes, BytesMut};
use memchr::memchr;

use super::{Decoder, Encoder};

/// Lines codec. Reads/writes line delimited strings.
///
/// Will split input up by LF or CRLF delimiters. Carriage return characters at the end of lines are
/// not preserved.
#[derive(Debug, Copy, Clone, Default)]
#[non_exhaustive]
pub struct LinesCodec;

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
            return Ok(None);
        }

        let len = match memchr(b'\n', src) {
            Some(n) => n,
            None => {
                return Ok(None);
            }
        };

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
}
