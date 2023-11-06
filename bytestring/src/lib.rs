//! A UTF-8 encoded read-only string using `Bytes` as storage.
//!
//! See docs for [`ByteString`].

#![no_std]
#![deny(rust_2018_idioms, nonstandard_style)]
#![warn(future_incompatible, missing_docs)]

extern crate alloc;

use alloc::{
    boxed::Box,
    string::{String, ToString},
    vec::Vec,
};
use core::{borrow::Borrow, convert::TryFrom, fmt, hash, ops, str};

use bytes::Bytes;

/// An immutable UTF-8 encoded string with [`Bytes`] as a storage.
#[derive(Clone, Default, Eq, PartialOrd, Ord)]
pub struct ByteString(Bytes);

impl ByteString {
    /// Creates a new empty `ByteString`.
    pub const fn new() -> Self {
        ByteString(Bytes::new())
    }

    /// Get a reference to the underlying `Bytes` object.
    pub fn as_bytes(&self) -> &Bytes {
        &self.0
    }

    /// Unwraps this `ByteString` into the underlying `Bytes` object.
    pub fn into_bytes(self) -> Bytes {
        self.0
    }

    /// Creates a new `ByteString` from a `&'static str`.
    pub const fn from_static(src: &'static str) -> ByteString {
        Self(Bytes::from_static(src.as_bytes()))
    }

    /// Creates a new `ByteString` from a Bytes.
    ///
    /// # Safety
    /// This function is unsafe because it does not check the bytes passed to it are valid UTF-8.
    /// If this constraint is violated, it may cause memory unsafety issues with future users of
    /// the `ByteString`, as we assume that `ByteString`s are valid UTF-8. However, the most likely
    /// issue is that the data gets corrupted.
    pub const unsafe fn from_bytes_unchecked(src: Bytes) -> ByteString {
        Self(src)
    }

    /// Returns a new byte string that is equivalent to the given `subset`.
    ///
    /// When processing a `ByteString` buffer with other tools, one often gets a `&str` which is in
    /// fact a slice of the original `ByteString`; i.e., a subset of it. This function turns that
    /// `&str` into another `ByteString`, as if one had sliced the `ByteString` with the offsets
    /// that correspond to `subset`.
    ///
    /// Corresponds to [`Bytes::slice_ref`].
    ///
    /// This operation is `O(1)`.
    ///
    /// # Panics
    ///
    /// Panics if `subset` is not a sub-slice of this byte string.
    ///
    /// Note that strings which are only subsets from an equality perspective do not uphold this
    /// requirement; see examples.
    ///
    /// # Examples
    ///
    /// ```
    /// # use bytestring::ByteString;
    /// let string = ByteString::from_static(" foo ");
    /// let subset = string.trim();
    /// let substring = string.slice_ref(subset);
    /// assert_eq!(substring, "foo");
    /// ```
    ///
    /// ```should_panic
    /// # use bytestring::ByteString;
    /// // panics because the given slice is not derived from the original byte string, despite
    /// // being a logical subset of the string
    /// ByteString::from_static("foo bar").slice_ref("foo");
    /// ```
    pub fn slice_ref(&self, subset: &str) -> Self {
        Self(self.0.slice_ref(subset.as_bytes()))
    }
}

impl PartialEq<str> for ByteString {
    fn eq(&self, other: &str) -> bool {
        &self[..] == other
    }
}

impl<T: AsRef<str>> PartialEq<T> for ByteString {
    fn eq(&self, other: &T) -> bool {
        &self[..] == other.as_ref()
    }
}

impl AsRef<ByteString> for ByteString {
    fn as_ref(&self) -> &ByteString {
        self
    }
}

impl AsRef<[u8]> for ByteString {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl AsRef<str> for ByteString {
    fn as_ref(&self) -> &str {
        self
    }
}

impl hash::Hash for ByteString {
    fn hash<H: hash::Hasher>(&self, state: &mut H) {
        (**self).hash(state);
    }
}

impl ops::Deref for ByteString {
    type Target = str;

    #[inline]
    fn deref(&self) -> &str {
        let bytes = self.0.as_ref();
        // SAFETY: UTF-8 validity is guaranteed during construction.
        unsafe { str::from_utf8_unchecked(bytes) }
    }
}

impl Borrow<str> for ByteString {
    fn borrow(&self) -> &str {
        self
    }
}

impl From<String> for ByteString {
    #[inline]
    fn from(value: String) -> Self {
        Self(Bytes::from(value))
    }
}

impl From<&str> for ByteString {
    #[inline]
    fn from(value: &str) -> Self {
        Self(Bytes::copy_from_slice(value.as_ref()))
    }
}

impl From<Box<str>> for ByteString {
    #[inline]
    fn from(value: Box<str>) -> Self {
        Self(Bytes::from(value.into_boxed_bytes()))
    }
}

impl From<ByteString> for String {
    #[inline]
    fn from(value: ByteString) -> Self {
        value.to_string()
    }
}

impl TryFrom<&[u8]> for ByteString {
    type Error = str::Utf8Error;

    #[inline]
    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        let _ = str::from_utf8(value)?;
        Ok(ByteString(Bytes::copy_from_slice(value)))
    }
}

impl TryFrom<Vec<u8>> for ByteString {
    type Error = str::Utf8Error;

    #[inline]
    fn try_from(value: Vec<u8>) -> Result<Self, Self::Error> {
        let buf = String::from_utf8(value).map_err(|err| err.utf8_error())?;
        Ok(ByteString(Bytes::from(buf)))
    }
}

impl TryFrom<Bytes> for ByteString {
    type Error = str::Utf8Error;

    #[inline]
    fn try_from(value: Bytes) -> Result<Self, Self::Error> {
        let _ = str::from_utf8(value.as_ref())?;
        Ok(ByteString(value))
    }
}

impl TryFrom<bytes::BytesMut> for ByteString {
    type Error = str::Utf8Error;

    #[inline]
    fn try_from(value: bytes::BytesMut) -> Result<Self, Self::Error> {
        let _ = str::from_utf8(&value)?;
        Ok(ByteString(value.freeze()))
    }
}

macro_rules! array_impls {
    ($($len:expr)+) => {
        $(
            impl TryFrom<[u8; $len]> for ByteString {
                type Error = str::Utf8Error;

                #[inline]
                fn try_from(value: [u8; $len]) -> Result<Self, Self::Error> {
                    ByteString::try_from(&value[..])
                }
            }

            impl TryFrom<&[u8; $len]> for ByteString {
                type Error = str::Utf8Error;

                #[inline]
                fn try_from(value: &[u8; $len]) -> Result<Self, Self::Error> {
                    ByteString::try_from(&value[..])
                }
            }
        )+
    }
}

array_impls!(0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30 31 32);

impl fmt::Debug for ByteString {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        (**self).fmt(fmt)
    }
}

impl fmt::Display for ByteString {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        (**self).fmt(fmt)
    }
}

#[cfg(feature = "serde")]
mod serde {
    use alloc::string::String;

    use serde::{
        de::{Deserialize, Deserializer},
        ser::{Serialize, Serializer},
    };

    use super::ByteString;

    impl Serialize for ByteString {
        #[inline]
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            serializer.serialize_str(self.as_ref())
        }
    }

    impl<'de> Deserialize<'de> for ByteString {
        #[inline]
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            String::deserialize(deserializer).map(ByteString::from)
        }
    }

    #[cfg(test)]
    mod serde_impl_tests {
        use serde::de::DeserializeOwned;
        use static_assertions::assert_impl_all;

        use super::*;

        assert_impl_all!(ByteString: Serialize, DeserializeOwned);
    }
}

#[cfg(test)]
mod test {
    use alloc::{borrow::ToOwned, format, vec};
    use core::{
        hash::{Hash, Hasher},
        panic::{RefUnwindSafe, UnwindSafe},
    };

    use ahash::AHasher;
    use static_assertions::assert_impl_all;

    use super::*;

    assert_impl_all!(ByteString: Send, Sync, Unpin, Sized);
    assert_impl_all!(ByteString: Clone, Default, Eq, PartialOrd, Ord);
    assert_impl_all!(ByteString: fmt::Debug, fmt::Display);
    assert_impl_all!(ByteString: UnwindSafe, RefUnwindSafe);

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
        let mut hasher1 = AHasher::default();
        "str".hash(&mut hasher1);

        let mut hasher2 = AHasher::default();
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
        let _ = ByteString::try_from(b"nice bytes").unwrap();
    }

    #[test]
    fn try_from_array() {
        assert_eq!(
            ByteString::try_from([b'h', b'i']).unwrap(),
            ByteString::from_static("hi")
        );
    }

    #[test]
    fn try_from_vec() {
        let _ = ByteString::try_from(vec![b'f', b'o', b'o']).unwrap();
        ByteString::try_from(vec![0, 159, 146, 150]).unwrap_err();
    }

    #[test]
    fn try_from_bytes() {
        let _ = ByteString::try_from(Bytes::from_static(b"nice bytes")).unwrap();
    }

    #[test]
    fn try_from_bytes_mut() {
        let _ = ByteString::try_from(bytes::BytesMut::from(&b"nice bytes"[..])).unwrap();
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
        ByteString::from_static("foo bar").slice_ref("foo");
    }
}
