//! Public API integration tests for ByteString.

use std::{
    borrow::Borrow,
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    ops::Bound,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use bytes::Bytes;
use bytestring::ByteString;

#[test]
fn eq() {
    let s: ByteString = ByteString::from_static("test");
    assert_eq!(s, "test");
    assert_eq!(s, *"test");
    assert_eq!(s, "test".to_owned());
}

#[test]
fn new() {
    let _: ByteString = ByteString::new();
}

#[test]
fn as_bytes() {
    let buf = ByteString::new();
    assert!(buf.as_bytes().is_empty());

    let buf = ByteString::from("hello");
    assert_eq!(buf.as_bytes(), "hello");
}

#[test]
fn into_bytes() {
    let buf = ByteString::from("hello");
    assert_eq!(buf.into_bytes(), Bytes::from_static(b"hello"));
}

#[test]
fn from_bytes_unchecked() {
    let buf = unsafe { ByteString::from_bytes_unchecked(Bytes::new()) };
    assert!(buf.is_empty());

    let buf = unsafe { ByteString::from_bytes_unchecked(Bytes::from("hello")) };
    assert_eq!(buf, "hello");
}

#[test]
fn as_ref() {
    let buf = ByteString::new();

    let _: &ByteString = buf.as_ref();
    let _: &[u8] = buf.as_ref();
}

#[test]
fn borrow() {
    let buf = ByteString::new();

    let _: &str = buf.borrow();
}

#[test]
fn hash() {
    let mut hasher1 = DefaultHasher::new();
    "str".hash(&mut hasher1);

    let mut hasher2 = DefaultHasher::new();
    let s = ByteString::from_static("str");
    s.hash(&mut hasher2);
    assert_eq!(hasher1.finish(), hasher2.finish());
}

#[test]
fn from_string() {
    let s: ByteString = "hello".to_owned().into();
    assert_eq!(&s, "hello");
    let t: &str = s.as_ref();
    assert_eq!(t, "hello");
}

#[test]
fn from_str() {
    let _: ByteString = "str".into();
    let _: ByteString = "str".to_owned().into_boxed_str().into();
}

#[test]
fn to_string() {
    let buf = ByteString::from("foo");
    assert_eq!(String::from(buf), "foo");
}

#[test]
fn from_static_str() {
    static _S: ByteString = ByteString::from_static("hello");
    let _ = ByteString::from_static("str");
}

#[test]
fn try_from_slice() {
    let heart = "\u{1f496}";
    let _ = ByteString::try_from(b"nice bytes").unwrap();
    assert_eq!(ByteString::try_from(heart.as_bytes()).unwrap(), heart);
    ByteString::try_from(&[0, 159, 146, 150][..]).unwrap_err();
}

#[test]
fn try_from_array() {
    assert_eq!(
        ByteString::try_from(*b"hi").unwrap(),
        ByteString::from_static("hi")
    );

    let empty = [];
    assert_eq!(ByteString::try_from(empty).unwrap(), "");
    assert_eq!(ByteString::try_from(&empty).unwrap(), "");

    let heart = [0xf0, 0x9f, 0x92, 0x96];
    assert_eq!(ByteString::try_from(heart).unwrap(), "\u{1f496}");
    assert_eq!(ByteString::try_from(&heart).unwrap(), "\u{1f496}");

    let invalid = [0, 159, 146, 150];
    ByteString::try_from(invalid).unwrap_err();
    ByteString::try_from(&invalid).unwrap_err();
}

#[test]
fn try_from_vec() {
    let _ = ByteString::try_from(vec![b'f', b'o', b'o']).unwrap();
    let heart = "\u{1f496}";
    assert_eq!(
        ByteString::try_from(heart.as_bytes().to_owned()).unwrap(),
        heart
    );
    ByteString::try_from(vec![0, 159, 146, 150]).unwrap_err();
}

#[test]
fn try_from_bytes() {
    let heart = "\u{1f496}";
    let _ = ByteString::try_from(Bytes::from_static(b"nice bytes")).unwrap();
    assert_eq!(
        ByteString::try_from(Bytes::from_static(heart.as_bytes())).unwrap(),
        heart
    );
    ByteString::try_from(Bytes::from_static(b"\x00\x9f\x92\x96")).unwrap_err();
}

#[test]
fn try_from_bytes_mut() {
    let heart = "\u{1f496}";
    let _ = ByteString::try_from(bytes::BytesMut::from(&b"nice bytes"[..])).unwrap();
    assert_eq!(
        ByteString::try_from(bytes::BytesMut::from(heart.as_bytes())).unwrap(),
        heart
    );
    ByteString::try_from(bytes::BytesMut::from(&b"\x00\x9f\x92\x96"[..])).unwrap_err();
}

#[test]
fn display() {
    let buf = ByteString::from("bar");
    assert_eq!(format!("{buf}"), "bar");
}

#[test]
fn debug() {
    let buf = ByteString::from("baz");
    assert_eq!(format!("{buf:?}"), r#""baz""#);
}

#[cfg(feature = "serde")]
#[test]
fn serialize() {
    let s: ByteString = serde_json::from_str(r#""nice bytes""#).unwrap();
    assert_eq!(s, "nice bytes");
}

#[cfg(feature = "serde")]
#[test]
fn deserialize() {
    let s = serde_json::to_string(&ByteString::from_static("nice bytes")).unwrap();
    assert_eq!(s, r#""nice bytes""#);
}

#[test]
fn slice_ref() {
    let string = ByteString::from_static(" foo ");
    let subset = string.trim();
    // subset is derived from original byte string
    let substring = string.slice_ref(subset);
    assert_eq!(substring, "foo");
}

#[test]
#[should_panic]
fn slice_ref_catches_not_a_subset() {
    // panics because the given slice is not derived from the original byte string, despite
    // being a logical subset of the string
    let _ = ByteString::from_static("foo bar").slice_ref("foo");
}

#[test]
fn split_at() {
    let buf = ByteString::from_static("foo bar");

    let (first, last) = buf.split_at(0);
    assert_eq!(ByteString::from_static(""), first);
    assert_eq!(ByteString::from_static("foo bar"), last);

    let (first, last) = buf.split_at(4);
    assert_eq!(ByteString::from_static("foo "), first);
    assert_eq!(ByteString::from_static("bar"), last);

    let (first, last) = buf.split_at(7);
    assert_eq!(ByteString::from_static("foo bar"), first);
    assert_eq!(ByteString::from_static(""), last);
}

#[test]
#[should_panic = "byte index 1 is not a char boundary;"]
fn split_at_invalid_code_point() {
    let _ = ByteString::from_static("µ").split_at(1);
}

#[test]
#[should_panic = "byte index 9 is out of bounds"]
fn split_at_outside_string() {
    let _ = ByteString::from_static("foo").split_at(9);
}

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

struct Owner {
    text: String,
    drops: Arc<AtomicUsize>,
}

impl AsRef<str> for Owner {
    fn as_ref(&self) -> &str {
        &self.text
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn owner_lives_until_last_shared_view_is_dropped() {
    let drops = Arc::new(AtomicUsize::new(0));
    let owner = Owner {
        text: String::from("aµz"),
        drops: drops.clone(),
    };
    let ptr = owner.text.as_ptr();
    let text = ByteString::from_owner(owner);
    assert_eq!(text.as_ptr(), ptr);
    let clone = text.clone();
    let part = text.slice_ref(&text[1..3]);
    drop(text);
    drop(clone);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    assert_eq!(part, "µ");
    drop(part);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn empty_owner_is_released() {
    let drops = Arc::new(AtomicUsize::new(0));
    let text = ByteString::from_owner(Owner {
        text: String::new(),
        drops: drops.clone(),
    });
    assert!(text.is_empty());
    drop(text);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn ranges_share_storage() {
    let text = ByteString::from("aµz");
    let part = text.slice(1..3);
    assert_eq!(part, "µ");
    assert_eq!(part.as_ptr(), text[1..].as_ptr());
    assert_eq!(text.slice(..), text);
    assert_eq!(text.slice(..=2), "aµ");
    assert_eq!(text.slice(3..), "z");
    assert_eq!(text.slice(4..4), "");
    assert_eq!(text.slice((Bound::Excluded(0), Bound::Excluded(3))), "µ");
    drop(text);
    assert_eq!(part, "µ");
}

#[test]
fn invalid_ranges_panic() {
    let text = ByteString::from("aµz");
    for (start, end) in [(2, 3), (0, 2), (3, 1), (0, 5)] {
        assert!(std::panic::catch_unwind(|| text.slice(start..end)).is_err());
    }
    assert!(std::panic::catch_unwind(|| text.slice(..=usize::MAX)).is_err());
    assert!(std::panic::catch_unwind(|| {
        text.slice((Bound::Excluded(usize::MAX), Bound::Unbounded))
    })
    .is_err());
}

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
