//! Public API tests for utf8 operations.

use bytes::Bytes;
use bytestring::ByteString;

#[test]
fn conversion_preserves_storage() {
    for text in ["", "ascii", "aµz"] {
        let bytes = Bytes::copy_from_slice(text.as_bytes());
        let ptr = bytes.as_ptr();
        let string = ByteString::from_utf8(bytes).unwrap();
        assert_eq!(string, text);
        assert_eq!(string.as_ptr(), ptr);
    }
}

#[test]
fn error_preserves_bytes_and_details() {
    for invalid in [&b"a\xff"[..], &b"a\xc2"[..]] {
        let bytes = Bytes::copy_from_slice(invalid);
        let ptr = bytes.as_ptr();
        let expected = std::str::from_utf8(invalid).unwrap_err();
        let err = ByteString::from_utf8(bytes).unwrap_err();
        let _: &dyn std::error::Error = &err;
        assert_eq!(err.as_bytes(), invalid);
        assert_eq!(err.utf8_error(), expected);
        assert_eq!(err.to_string(), expected.to_string());
        let recovered = err.into_bytes();
        assert_eq!(recovered.as_ptr(), ptr);
        assert_eq!(recovered.as_ref(), invalid);
    }
}

#[test]
fn try_from_keeps_existing_error_type() {
    let result: Result<ByteString, std::str::Utf8Error> =
        ByteString::try_from(Bytes::from_static(b"\xff"));
    assert!(result.is_err());
}
