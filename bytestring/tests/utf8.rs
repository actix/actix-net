//! Comprehensive UTF-8 validation and error-fidelity tests for `ByteString`.

use std::str;

use bytes::{Bytes, BytesMut};
use bytestring::ByteString;

fn assert_valid_all(input: &[u8]) {
    let expected = str::from_utf8(input).expect("test case must be valid UTF-8");

    let bytes = Bytes::copy_from_slice(input);
    let ptr = bytes.as_ptr();
    let from_utf8 = ByteString::from_utf8(bytes).expect("from_utf8 should succeed");
    assert_eq!(from_utf8.as_str(), expected);
    if !input.is_empty() {
        assert_eq!(from_utf8.as_ptr(), ptr);
    }

    assert_eq!(
        ByteString::try_from(input)
            .expect("TryFrom<&[u8]> should succeed")
            .as_str(),
        expected
    );

    assert_eq!(
        ByteString::try_from(input.to_vec())
            .expect("TryFrom<Vec<u8>> should succeed")
            .as_str(),
        expected
    );

    assert_eq!(
        ByteString::try_from(Bytes::copy_from_slice(input))
            .expect("TryFrom<Bytes> should succeed")
            .as_str(),
        expected
    );

    assert_eq!(
        ByteString::try_from(BytesMut::from(input))
            .expect("TryFrom<BytesMut> should succeed")
            .as_str(),
        expected
    );
}

fn assert_invalid_all(input: &[u8]) {
    let expected = str::from_utf8(input).expect_err("test case must be invalid UTF-8");

    let bytes = Bytes::copy_from_slice(input);
    let ptr = bytes.as_ptr();
    let from_utf8_err = ByteString::from_utf8(bytes).expect_err("from_utf8 should fail");
    assert_eq!(from_utf8_err.utf8_error(), expected);
    assert_eq!(
        from_utf8_err.utf8_error().valid_up_to(),
        expected.valid_up_to()
    );
    assert_eq!(from_utf8_err.utf8_error().error_len(), expected.error_len());
    assert_eq!(from_utf8_err.to_string(), expected.to_string());
    let recovered = from_utf8_err.into_bytes();
    assert_eq!(recovered.as_ptr(), ptr);
    assert_eq!(recovered.as_ref(), input);

    let slice_err = ByteString::try_from(input).expect_err("TryFrom<&[u8]> should fail");
    assert_eq!(slice_err, expected);
    assert_eq!(slice_err.valid_up_to(), expected.valid_up_to());
    assert_eq!(slice_err.error_len(), expected.error_len());

    let vec_err = ByteString::try_from(input.to_vec()).expect_err("TryFrom<Vec<u8>> should fail");
    assert_eq!(vec_err, expected);
    assert_eq!(vec_err.valid_up_to(), expected.valid_up_to());
    assert_eq!(vec_err.error_len(), expected.error_len());

    let bytes_err = ByteString::try_from(Bytes::copy_from_slice(input))
        .expect_err("TryFrom<Bytes> should fail");
    assert_eq!(bytes_err, expected);
    assert_eq!(bytes_err.valid_up_to(), expected.valid_up_to());
    assert_eq!(bytes_err.error_len(), expected.error_len());

    let bytes_mut_err =
        ByteString::try_from(BytesMut::from(input)).expect_err("TryFrom<BytesMut> should fail");
    assert_eq!(bytes_mut_err, expected);
    assert_eq!(bytes_mut_err.valid_up_to(), expected.valid_up_to());
    assert_eq!(bytes_mut_err.error_len(), expected.error_len());
}

fn repeat_multibyte(target_len: usize) -> Vec<u8> {
    const PATTERN: &[u8] = "café世界🚀".as_bytes();
    let mut buf = Vec::with_capacity(target_len + PATTERN.len());
    while buf.len() + PATTERN.len() <= target_len {
        buf.extend_from_slice(PATTERN);
    }
    while buf.len() + 2 <= target_len {
        buf.extend_from_slice("é".as_bytes());
    }
    if buf.len() < target_len {
        buf.push(b'a');
    }
    assert_eq!(buf.len(), target_len);
    buf
}

#[test]
fn valid_ascii_various_lengths() {
    for len in [
        0, 1, 2, 7, 8, 15, 16, 31, 32, 62, 63, 64, 65, 66, 127, 128, 129, 255, 256, 1024, 65_536,
    ] {
        let buf = vec![b'x'; len];
        assert_valid_all(&buf);
    }
}

#[test]
fn valid_boundary_codepoints() {
    let boundary_str =
        "\u{0000}\u{007f}\u{0080}\u{07ff}\u{0800}\u{d7ff}\u{e000}\u{ffff}\u{10000}\u{10ffff}";
    assert_valid_all(boundary_str.as_bytes());

    // Pad with ASCII before and after to exercise SIMD chunk paths as well.
    let mut padded = vec![b'a'; 63];
    padded.extend_from_slice(boundary_str.as_bytes());
    padded.extend_from_slice(&[b'z'; 65]);
    assert_valid_all(&padded);
}

#[test]
fn valid_multibyte_straddling_simd_chunk_boundaries() {
    for sample in ["é", "世", "🚀"] {
        for prefix_len in [
            0, 1, 15, 16, 31, 32, 60, 61, 62, 63, 64, 65, 125, 126, 127, 128,
        ] {
            for suffix_len in [0, 1, 16, 64] {
                let mut ascii_prefix = vec![b'a'; prefix_len];
                ascii_prefix.extend_from_slice(sample.as_bytes());
                ascii_prefix.extend(std::iter::repeat_n(b'b', suffix_len));
                assert_valid_all(&ascii_prefix);

                let mut mb_prefix = repeat_multibyte(prefix_len);
                mb_prefix.extend_from_slice(sample.as_bytes());
                mb_prefix.extend(std::iter::repeat_n(b'b', suffix_len));
                assert_valid_all(&mb_prefix);
            }
        }
    }
}

#[test]
fn valid_large_payloads() {
    for size in [1024, 65_536] {
        // ASCII with late non-ASCII char at the very end.
        let mut late = vec![b'a'; size - 3];
        late.extend_from_slice("世".as_bytes());
        assert_valid_all(&late);

        // Dense multi-byte payload.
        let dense = repeat_multibyte(size);
        assert_valid_all(&dense);
    }
}

#[test]
fn invalid_sequences_exact_utf8_error() {
    let invalid_samples: &[&[u8]] = &[
        // Lone continuation / invalid starter bytes
        b"\x80",
        b"\xbf",
        b"\xc0",
        b"\xc1",
        b"\xf5",
        b"\xff",
        // Overlong encodings
        b"\xc0\x80",
        b"\xc0\xaf",
        b"\xc1\xbf",
        b"\xe0\x80\x80",
        b"\xe0\x9f\xbf",
        b"\xf0\x80\x80\x80",
        b"\xf0\x8f\xbf\xbf",
        // UTF-16 surrogates (U+D800..=U+DFFF)
        b"\xed\xa0\x80",
        b"\xed\xad\xbf",
        b"\xed\xbf\xbf",
        // Above U+10FFFF
        b"\xf4\x90\x80\x80",
        b"\xf4\xbf\xbf\xbf",
        // Truncated sequences at EOF (error_len() == None)
        b"\xc2",
        b"\xe2",
        b"\xe2\x82",
        b"\xf0",
        b"\xf0\x9f",
        b"\xf0\x9f\x92",
        // Truncated sequences followed by ASCII (error_len() == Some(1|2|3))
        b"\xc2a",
        b"\xe2a",
        b"\xe2\x82a",
        b"\xf0a",
        b"\xf0\x9fa",
        b"\xf0\x9f\x92a",
        // Valid char immediately followed by invalid sequence
        b"\xc3\xa9\x80",
        b"\xe4\xb8\x96\xed\xa0\x80",
        b"\xf0\x9f\x9a\x80\xf4\x90\x80\x80",
    ];

    let prefix_lens = [0, 1, 5, 61, 62, 63, 64, 65, 127, 128, 256];
    let suffix_lens = [0, 1, 16, 70];

    for &sample in invalid_samples {
        for &prefix_len in &prefix_lens {
            for &suffix_len in &suffix_lens {
                // Pure ASCII prefix
                let mut buf = vec![b'a'; prefix_len];
                buf.extend_from_slice(sample);
                buf.extend(std::iter::repeat_n(b'z', suffix_len));
                assert_invalid_all(&buf);

                // Dense multi-byte prefix
                let mut mb = repeat_multibyte(prefix_len);
                mb.extend_from_slice(sample);
                mb.extend(std::iter::repeat_n(b'z', suffix_len));
                assert_invalid_all(&mb);
            }
        }
    }
}

#[test]
fn invalid_sequences_in_large_buffers() {
    for size in [1024, 65_536] {
        // Early invalid byte in large ASCII buffer
        let mut early = vec![b'a'; size];
        early[4] = 0xff;
        assert_invalid_all(&early);

        // Late invalid byte after pure ASCII prefix
        let mut late_ascii = vec![b'a'; size];
        late_ascii[size - 1] = 0xff;
        assert_invalid_all(&late_ascii);

        // Late incomplete 3-byte sequence at EOF after pure ASCII prefix (error_len == None)
        let mut late_incomplete_ascii = vec![b'a'; size - 2];
        late_incomplete_ascii.extend_from_slice(&[0xe2, 0x82]);
        assert_invalid_all(&late_incomplete_ascii);

        // Late invalid byte after dense multi-byte prefix
        let mut late_mb = repeat_multibyte(size - 1);
        late_mb.push(0xff);
        assert_invalid_all(&late_mb);

        // Late invalid 4-byte sequence (error_len == Some(3)) after dense multi-byte prefix
        let mut late_mb_3 = repeat_multibyte(size - 4);
        late_mb_3.extend_from_slice(b"\xf0\x9f\x92a");
        assert_invalid_all(&late_mb_3);
    }
}
