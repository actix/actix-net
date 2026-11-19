//! Preserve line contents, errors, and remaining input during allocation experiments.
use actix_codec::{Decoder, LinesCodec};
use bytes::BytesMut;

#[test]
fn decoded_lines_survive_reusing_input() {
    let mut codec = LinesCodec::new();
    let mut src = BytesMut::from(&b"one\r\n\nthree\n"[..]);
    let one = codec.decode(&mut src).unwrap().unwrap();
    assert_eq!(codec.decode(&mut src).unwrap().unwrap(), "");
    assert_eq!(codec.decode(&mut src).unwrap().unwrap(), "three");
    src.clear();
    src.extend_from_slice(b"other\n");
    assert_eq!(codec.decode(&mut src).unwrap().unwrap(), "other");
    assert_eq!(one, "one");
}

#[test]
fn invalid_utf8_consumes_only_the_invalid_line() {
    let mut codec = LinesCodec::new();
    let mut src = BytesMut::from(&b"\xff\r\ngood\n"[..]);
    assert_eq!(
        codec.decode(&mut src).unwrap_err().kind(),
        std::io::ErrorKind::InvalidData
    );
    assert_eq!(&src[..], b"good\n");
    assert_eq!(codec.decode(&mut src).unwrap().unwrap(), "good");
}

#[test]
fn eof_preserves_trailing_carriage_return() {
    let mut codec = LinesCodec::new();
    let mut src = BytesMut::from(&b"last\r"[..]);
    assert_eq!(codec.decode_eof(&mut src).unwrap().unwrap(), "last");
    assert_eq!(&src[..], b"\r");
    assert!(codec.decode_eof(&mut src).unwrap().is_none());
}

#[test]
fn line_limit_accepts_crlf_and_preserves_overlong_input() {
    let mut codec = LinesCodec::new_with_max_length(4);
    let mut src = BytesMut::from(&b"four\r\nlonger\n"[..]);
    assert_eq!(codec.decode(&mut src).unwrap().unwrap(), "four");
    assert_eq!(
        codec.decode(&mut src).unwrap_err().kind(),
        std::io::ErrorKind::InvalidData
    );
    assert_eq!(&src[..], b"longer\n");
}

// A stateless model keeps this check independent of the decoder's search cursor and buffer splitting.
fn reference(
    src: &mut Vec<u8>,
    max: usize,
    eof: bool,
) -> Result<Option<String>, std::io::ErrorKind> {
    use std::io::ErrorKind::InvalidData;
    let newline = src.iter().position(|&byte| byte == b'\n');
    let end = newline.unwrap_or(src.len());
    let len = end - usize::from(src[..end].last() == Some(&b'\r'));
    if len > max {
        return Err(InvalidData);
    }
    if newline.is_none() && !eof {
        return Ok(None);
    }
    if newline.is_none() && len == 0 {
        return Ok(None);
    }
    let line = String::from_utf8(src[..len].to_vec()).map_err(|_| InvalidData);
    let consumed = newline.map_or(len, |index| index + 1);
    src.drain(..consumed);
    line.map(Some)
}

#[test]
fn exhaustive_short_inputs_match_reference() {
    let alphabet = [b'a', b'\n', b'\r', 0xc3, 0xa9, 0xff];
    for len in 0..=6 {
        for mut code in 0..6_usize.pow(len) {
            let input: Vec<_> = (0..len)
                .map(|_| {
                    let byte = alphabet[code % 6];
                    code /= 6;
                    byte
                })
                .collect();
            for max in [0, 1, 3, usize::MAX] {
                for chunk in [1, 2, 5] {
                    let mut codec = LinesCodec::new_with_max_length(max);
                    let mut src = BytesMut::new();
                    let mut expected = Vec::new();
                    for part in input.chunks(chunk) {
                        src.extend_from_slice(part);
                        expected.extend_from_slice(part);
                        loop {
                            let actual = codec.decode(&mut src).map_err(|err| err.kind());
                            let wanted = reference(&mut expected, max, false);
                            assert_eq!(actual, wanted, "input={input:?}, max={max}, chunk={chunk}");
                            assert_eq!(&src[..], expected);
                            if !matches!(actual, Ok(Some(_))) {
                                break;
                            }
                        }
                    }
                    for _ in 0..2 {
                        let actual = codec.decode_eof(&mut src).map_err(|err| err.kind());
                        assert_eq!(
                            actual,
                            reference(&mut expected, max, true),
                            "EOF input={input:?}, max={max}, chunk={chunk}"
                        );
                        assert_eq!(&src[..], expected);
                    }
                }
            }
        }
    }
}

#[test]
fn large_unicode_lines_survive_fragmentation_and_shared_input() {
    for size in [32, 4096, 524288] {
        let line = "é".repeat(size);
        for chunk in [1, 63, 8192] {
            let mut codec = LinesCodec::new_with_max_length(line.len());
            let mut src = BytesMut::from(&b"prefix"[..]);
            let prefix = src.split_to(6).freeze();
            for part in line.as_bytes().chunks(chunk) {
                src.extend_from_slice(part);
                assert!(codec.decode(&mut src).unwrap().is_none());
            }
            src.extend_from_slice(b"\r\nnext\n");
            let decoded = codec.decode(&mut src).unwrap().unwrap();
            assert_eq!(decoded, line);
            assert_eq!(codec.decode(&mut src).unwrap().unwrap(), "next");
            src.extend_from_slice(b"reuse\n");
            assert_eq!(codec.decode(&mut src).unwrap().unwrap(), "reuse");
            assert_eq!(decoded, line);
            assert_eq!(&prefix[..], b"prefix");
        }
    }
}

#[test]
fn invalid_utf8_error_retains_rejected_bytes() {
    for eof in [false, true] {
        let mut codec = LinesCodec::new();
        let mut src = BytesMut::from(if eof {
            &b"\xff\r"[..]
        } else {
            &b"\xff\r\nnext\n"[..]
        });
        let error = if eof {
            codec.decode_eof(&mut src)
        } else {
            codec.decode(&mut src)
        }
        .unwrap_err();
        let utf8 = error
            .get_ref()
            .unwrap()
            .downcast_ref::<std::string::FromUtf8Error>()
            .unwrap();
        assert_eq!(utf8.as_bytes(), b"\xff");
        assert_eq!(&src[..], if eof { &b"\r"[..] } else { &b"next\n"[..] });
    }
}
