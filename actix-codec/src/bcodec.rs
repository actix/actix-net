use std::io;

use bytes::{Buf, Bytes, BytesMut};

use super::{Decoder, Encoder};

/// Bytes codec. Reads/writes chunks of bytes from a stream.
#[derive(Debug, Copy, Clone)]
pub struct BytesCodec;

impl Encoder<Bytes> for BytesCodec {
    type Error = io::Error;

    #[inline]
    fn encode(&mut self, item: Bytes, dst: &mut BytesMut) -> Result<(), Self::Error> {
        dst.extend_from_slice(item.chunk());
        Ok(())
    }
}

impl Decoder for BytesCodec {
    type Item = BytesMut;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.is_empty() {
            Ok(None)
        } else {
            Ok(Some(src.split()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_codec_appends_and_decodes_all_bytes() {
        let mut codec = BytesCodec;
        let mut buf = BytesMut::from(&b"prefix"[..]);

        codec
            .encode(Bytes::from_static(b"\0\xff"), &mut buf)
            .unwrap();
        codec.encode(Bytes::new(), &mut buf).unwrap();
        assert_eq!(
            codec.decode(&mut buf).unwrap().unwrap(),
            &b"prefix\0\xff"[..]
        );
        assert!(buf.is_empty());
        assert!(codec.decode(&mut buf).unwrap().is_none());

        buf.extend_from_slice(b"last");

        assert_eq!(codec.decode_eof(&mut buf).unwrap().unwrap(), "last");
        assert!(codec.decode_eof(&mut buf).unwrap().is_none());
    }
}
