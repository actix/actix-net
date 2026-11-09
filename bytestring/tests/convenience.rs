//! Public API tests for convenience operations.

use bytes::Bytes;
use bytestring::ByteString;

#[test]
fn length_queries_work_in_const_contexts() {
    const QUERIES: (usize, bool, bool) = {
        let text = ByteString::from_static("µ");
        let empty = ByteString::new();
        let result = (text.len(), empty.is_empty(), text.is_empty());

        std::mem::forget(text);
        std::mem::forget(empty);
        result
    };
    let (len, empty, nonempty) = QUERIES;
    assert_eq!(len, 2);
    assert!(empty);
    assert!(!nonempty);
}

#[test]
fn access_and_conversion_preserve_storage() {
    let text = ByteString::from("aµz");
    let ptr = text.as_ptr();
    assert_eq!(text.as_str(), "aµz");
    assert_eq!(text.as_str().as_ptr(), ptr);
    let bytes: Bytes = text.into();
    assert_eq!(bytes.as_ptr(), ptr);
    assert_eq!(bytes.as_ref(), "aµz".as_bytes());
}
