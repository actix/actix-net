//! Public API tests for cursor operations.

use bytestring::ByteString;

#[test]
fn split_operations_share_storage() {
    for at in [0, 1, 3, 4] {
        let original = ByteString::from("aµz");
        let mut prefix = original.clone();
        let suffix = prefix.split_off(at);
        assert_eq!(prefix, &original[..at]);
        assert_eq!(suffix, &original[at..]);
        if at < original.len() {
            assert_eq!(suffix.as_ptr(), original[at..].as_ptr());
        }
        let mut tail = original.clone();
        let head = tail.split_to(at);
        assert_eq!(head, prefix);
        assert_eq!(tail, suffix);
    }
}

#[test]
fn discard_operations_handle_boundaries() {
    for at in [0, 1, 3, 4] {
        let mut text = ByteString::from("aµz");
        text.advance(at);
        assert_eq!(text, &"aµz"[at..]);
        let mut text = ByteString::from("aµz");
        text.truncate(at);
        assert_eq!(text, &"aµz"[..at]);
    }
    let mut text = ByteString::from("aµz");
    text.truncate(usize::MAX);
    assert_eq!(text, "aµz");
}

#[test]
fn invalid_operations_leave_string_unchanged() {
    for at in [2, 5, usize::MAX] {
        for operation in 0..4 {
            if operation == 3 && at > 4 {
                continue;
            }
            let mut text = ByteString::from("aµz");
            let result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match operation {
                    0 => {
                        let _ = text.split_to(at);
                    }
                    1 => {
                        let _ = text.split_off(at);
                    }
                    2 => text.advance(at),
                    _ => text.truncate(at),
                }));
            assert!(result.is_err());
            assert_eq!(text, "aµz");
        }
    }
}
