use alloc::string::String;
use core::{borrow::Borrow, cmp::Ordering, fmt, hash, ops::Deref, str};

use bytes::Bytes;

use crate::ByteString;

// This capacity keeps SmallByteString the same size as ByteString on 64-bit targets.
const INLINE_CAPACITY: usize = 23;

/// An immutable UTF-8 string that can store short strings inline.
///
/// Converting a string slice copies its contents. Short slices fit inside the value,
/// and longer slices use a shared [`ByteString`]. Converting an owned [`String`] or
/// [`ByteString`] preserves its backing storage, even for short strings.
///
/// # Examples
/// ```
/// use bytestring::SmallByteString;
///
/// let field = {
///     let request = String::from("name: Alice");
///     SmallByteString::from(request.strip_prefix("name: ").unwrap())
/// };
/// assert_eq!(field, "Alice");
/// ```
#[derive(Clone)]
pub struct SmallByteString(Storage);

#[derive(Clone)]
enum Storage {
    Inline {
        len: u8,
        data: [u8; INLINE_CAPACITY],
    },
    Shared(ByteString),
}

impl SmallByteString {
    /// Creates an empty string.
    #[must_use]
    pub const fn new() -> Self {
        Self(Storage::Inline {
            len: 0,
            data: [0; INLINE_CAPACITY],
        })
    }

    /// Creates a string that borrows static storage without copying.
    #[must_use]
    pub const fn from_static(value: &'static str) -> Self {
        Self(Storage::Shared(ByteString::from_static(value)))
    }

    /// Borrows the string contents.
    #[inline]
    #[must_use]
    pub fn as_str(&self) -> &str {
        match &self.0 {
            Storage::Inline { len, data } => {
                // SAFETY: The initialized prefix is copied from a complete str.
                unsafe { str::from_utf8_unchecked(&data[..usize::from(*len)]) }
            }
            Storage::Shared(value) => value.as_str(),
        }
    }

    /// Returns the underlying bytes. This copies inline contents and reuses shared storage.
    #[must_use]
    pub fn into_bytes(self) -> Bytes {
        ByteString::from(self).into_bytes()
    }
}

impl Default for SmallByteString {
    fn default() -> Self {
        Self::new()
    }
}

impl From<&str> for SmallByteString {
    fn from(value: &str) -> Self {
        if value.len() > INLINE_CAPACITY {
            return Self::from(ByteString::from(value));
        }

        let mut data = [0; INLINE_CAPACITY];
        data[..value.len()].copy_from_slice(value.as_bytes());
        Self(Storage::Inline {
            len: value.len() as u8,
            data,
        })
    }
}

impl From<ByteString> for SmallByteString {
    fn from(value: ByteString) -> Self {
        Self(Storage::Shared(value))
    }
}

impl From<String> for SmallByteString {
    fn from(value: String) -> Self {
        Self::from(ByteString::from(value))
    }
}

impl From<SmallByteString> for ByteString {
    fn from(value: SmallByteString) -> Self {
        match value.0 {
            Storage::Inline { .. } => Self::from(value.as_str()),
            Storage::Shared(shared) => shared,
        }
    }
}

impl From<SmallByteString> for Bytes {
    fn from(value: SmallByteString) -> Self {
        value.into_bytes()
    }
}

impl Deref for SmallByteString {
    type Target = str;

    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<str> for SmallByteString {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<[u8]> for SmallByteString {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl Borrow<str> for SmallByteString {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl<T: AsRef<str> + ?Sized> PartialEq<T> for SmallByteString {
    fn eq(&self, other: &T) -> bool {
        self.as_str() == other.as_ref()
    }
}

impl Eq for SmallByteString {}

impl PartialOrd for SmallByteString {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SmallByteString {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_str().cmp(other.as_str())
    }
}

impl hash::Hash for SmallByteString {
    fn hash<H: hash::Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}

impl fmt::Display for SmallByteString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self.as_str(), f)
    }
}

impl fmt::Debug for SmallByteString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_pointer_width = "64")]
    static_assertions::assert_eq_size!(SmallByteString, ByteString);

    #[test]
    fn inline_boundary_uses_utf8_byte_length() {
        let limit = "é".repeat(11) + "a";
        let longer = limit.clone() + "a";

        let inline = SmallByteString::from(limit.as_str());
        let shared = SmallByteString::from(longer.as_str());
        assert!(matches!(inline.0, Storage::Inline { .. }));
        assert!(matches!(shared.0, Storage::Shared(_)));
        assert_eq!(inline.clone().into_bytes(), limit.as_bytes());
        assert_eq!(shared.clone().as_ptr(), shared.as_ptr());
        assert_eq!(shared.into_bytes(), longer.as_bytes());
    }

    #[test]
    fn preserves_short_owned_storage() {
        let owned = String::from("aµz");
        let ptr = owned.as_ptr();
        let small = SmallByteString::from(ByteString::from(owned));
        assert_eq!(small.clone().as_ptr(), ptr);
        assert_eq!(small.into_bytes().as_ptr(), ptr);
    }

    #[test]
    fn compares_inline_and_shared_contents() {
        let inline = SmallByteString::from("apple");
        let shared = SmallByteString::from_static("apple");
        assert_eq!(inline, shared);
        assert_eq!(inline.cmp(&shared), Ordering::Equal);
        assert!(inline < SmallByteString::from_static("pear"));
        assert_eq!(SmallByteString::default(), "");
    }
}
