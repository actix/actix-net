//! A UTF-8 encoded read-only string using `Bytes` as storage.
//!
//! See docs for [`ByteString`].

#![no_std]

extern crate alloc;

#[cfg(feature = "serde")]
mod serde;

use alloc::{boxed::Box, string::String, vec::Vec};
use core::{borrow::Borrow, fmt, hash, ops, str};

use bytes::Bytes;

/// An immutable UTF-8 encoded string using [`Bytes`] as the storage.
#[doc(alias = "BytesStr")]
#[doc(alias = "ByteStr")]
#[doc(alias = "Str")]
#[doc(alias = "String")]
#[doc(alias = "SharedString")]
#[doc(alias = "SharedStr")]
#[doc(alias = "ArcStr")]
#[doc(alias = "RcStr")]
#[doc(alias = "ImmutableString")]
#[doc(alias = "Utf8String")]
#[doc(alias = "Utf8Bytes")]
#[doc(alias = "UTF-8")]
#[doc(alias = "byte string")]
#[doc(alias = "shared string")]
#[doc(alias = "reference counted string")]
#[doc(alias = "immutable string")]
#[doc(alias = "read-only string")]
#[doc(alias = "zero-copy string")]
#[derive(Clone, Default, Eq, PartialOrd, Ord)]
pub struct ByteString(Bytes);

impl ByteString {
    /// Creates a new empty `ByteString`.
    ///
    /// # Examples
    /// ```
    /// # use bytestring::ByteString;
    /// let text = ByteString::new();
    /// assert_eq!(text, "");
    /// ```
    #[doc(alias = "empty")]
    #[inline]
    #[must_use]
    pub const fn new() -> Self {
        ByteString(Bytes::new())
    }

    /// Creates a new `ByteString` from a `&'static str`.
    ///
    /// # Examples
    /// ```
    /// # use bytestring::ByteString;
    /// let text = ByteString::from_static("hello");
    /// assert_eq!(text, "hello");
    /// ```
    #[doc(alias = "from_static_str")]
    #[inline]
    #[must_use]
    pub const fn from_static(src: &'static str) -> ByteString {
        Self(Bytes::from_static(src.as_bytes()))
    }

    /// Creates a string whose storage remains owned by `owner`.
    ///
    /// The contents are not copied. The owner is dropped when the last shared
    /// view of the storage is dropped. See [`Bytes::from_owner`].
    ///
    /// # Examples
    /// ```
    /// # use bytestring::ByteString;
    /// use std::sync::Arc;
    ///
    /// let text = ByteString::from_owner(Arc::<str>::from("hello"));
    /// assert_eq!(text, "hello");
    /// ```
    #[must_use]
    pub fn from_owner<T>(owner: T) -> Self
    where
        T: AsRef<str> + Send + 'static,
    {
        struct Owner<T>(T);

        impl<T: AsRef<str>> AsRef<[u8]> for Owner<T> {
            fn as_ref(&self) -> &[u8] {
                self.0.as_ref().as_bytes()
            }
        }

        // The owner's string reference guarantees that the bytes are valid UTF-8.
        Self(Bytes::from_owner(Owner(owner)))
    }

    /// Converts bytes to a string, retaining the bytes if UTF-8 validation fails.
    ///
    /// # Errors
    /// Returns [`FromUtf8Error`] if the bytes are not valid UTF-8.
    ///
    /// # Examples
    /// ```
    /// use bytes::Bytes;
    ///
    /// # use bytestring::ByteString;
    /// let err = ByteString::from_utf8(Bytes::from_static(b"hello\xff")).unwrap_err();
    /// assert_eq!(err.utf8_error().valid_up_to(), 5);
    /// assert_eq!(err.into_bytes(), Bytes::from_static(b"hello\xff"));
    /// ```
    #[inline]
    #[must_use]
    pub fn from_utf8(bytes: Bytes) -> Result<Self, FromUtf8Error> {
        match str::from_utf8(&bytes) {
            Ok(_) => Ok(Self(bytes)),
            Err(error) => Err(FromUtf8Error { bytes, error }),
        }
    }

    /// Creates a new `ByteString` from a Bytes.
    ///
    /// # Safety
    /// This function is unsafe because it does not check the bytes passed to it are valid UTF-8.
    /// If this constraint is violated, it may cause memory unsafety issues with future users of
    /// the `ByteString`, as we assume that `ByteString`s are valid UTF-8. However, the most likely
    /// issue is that the data gets corrupted.
    ///
    /// # Examples
    /// ```
    /// # use bytestring::ByteString;
    /// let bytes = bytes::Bytes::from_static(b"hello");
    ///
    /// // SAFETY: The bytes contain valid UTF-8.
    /// let text = unsafe { ByteString::from_bytes_unchecked(bytes) };
    /// assert_eq!(text, "hello");
    /// ```
    #[doc(alias = "from_utf8_unchecked")]
    #[inline]
    #[must_use]
    pub const unsafe fn from_bytes_unchecked(src: Bytes) -> ByteString {
        Self(src)
    }

    /// Returns the length in bytes.
    ///
    /// # Examples
    /// ```
    /// # use bytestring::ByteString;
    /// let text = ByteString::from("hello");
    /// assert_eq!(text.len(), 5);
    /// ```
    #[inline]
    #[must_use]
    pub const fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns true if this string is empty.
    ///
    /// # Examples
    /// ```
    /// # use bytestring::ByteString;
    /// let text = ByteString::new();
    /// assert!(text.is_empty());
    /// ```
    #[inline]
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Returns the string contents as a string slice.
    ///
    /// # Examples
    /// ```
    /// # use bytestring::ByteString;
    /// let text = ByteString::from("hello");
    /// assert_eq!(text.as_str(), "hello");
    /// ```
    #[inline]
    #[must_use]
    pub fn as_str(&self) -> &str {
        self
    }

    /// Get a reference to the underlying `Bytes` object.
    ///
    /// # Examples
    /// ```
    /// # use bytestring::ByteString;
    /// let text = ByteString::from("hello");
    /// assert_eq!(text.as_bytes(), &bytes::Bytes::from_static(b"hello"));
    /// ```
    #[doc(alias = "as_inner")]
    #[doc(alias = "as_ref")]
    #[inline]
    #[must_use]
    pub const fn as_bytes(&self) -> &Bytes {
        &self.0
    }

    /// Returns a shared slice for the given byte range.
    ///
    /// This operation does not copy the string contents.
    ///
    /// # Panics
    /// Panics if the range is out of bounds, reversed, or does not start and end
    /// on UTF-8 code point boundaries.
    ///
    /// # Examples
    /// ```
    /// # use bytestring::ByteString;
    /// let text = ByteString::from("aµz");
    /// assert_eq!(text.slice(1..3), "µ");
    /// ```
    #[must_use]
    pub fn slice(&self, range: impl ops::RangeBounds<usize>) -> Self {
        use ops::Bound;

        let start = match range.start_bound() {
            Bound::Included(&n) => n,
            Bound::Excluded(&n) => n.checked_add(1).expect("range start overflow"),
            Bound::Unbounded => 0,
        };

        let end = match range.end_bound() {
            Bound::Included(&n) => n.checked_add(1).expect("range end overflow"),
            Bound::Excluded(&n) => n,
            Bound::Unbounded => self.len(),
        };

        self.slice_ref(&self[start..end])
    }

    /// Returns a new `ByteString` that is equivalent to the given `subset`.
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
    /// Panics if `subset` is not a sub-slice of this byte string.
    ///
    /// Note that strings which are only subsets from an equality perspective do not uphold this
    /// requirement; see examples.
    ///
    /// # Examples
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
    /// let _ = ByteString::from_static("foo bar").slice_ref("foo");
    /// ```
    #[doc(alias = "slice")]
    #[doc(alias = "substring")]
    #[doc(alias = "substr")]
    #[doc(alias = "subslice")]
    #[inline]
    #[must_use]
    pub fn slice_ref(&self, subset: &str) -> Self {
        Self(self.0.slice_ref(subset.as_bytes()))
    }

    /// Splits off the suffix at the given byte index, retaining the prefix.
    ///
    /// The returned string shares storage with this string.
    ///
    /// # Panics
    /// Panics if `at` is out of bounds or is not a UTF-8 code point boundary.
    ///
    /// # Examples
    /// ```
    /// # use bytestring::ByteString;
    /// let mut text = ByteString::from("hello world");
    /// let suffix = text.split_off(5);
    ///
    /// assert_eq!(text, "hello");
    /// assert_eq!(suffix, " world");
    /// ```
    #[inline]
    #[must_use = "use truncate if the suffix is not needed"]
    pub fn split_off(&mut self, at: usize) -> Self {
        assert!(self.is_char_boundary(at), "invalid split boundary");

        Self(self.0.split_off(at))
    }

    /// Splits off the prefix at the given byte index, retaining the suffix.
    ///
    /// The returned string shares storage with this string.
    ///
    /// # Panics
    /// Panics if `at` is out of bounds or is not a UTF-8 code point boundary.
    ///
    /// # Examples
    /// ```
    /// # use bytestring::ByteString;
    /// let mut text = ByteString::from("hello world");
    /// let prefix = text.split_to(5);
    ///
    /// assert_eq!(text, " world");
    /// assert_eq!(prefix, "hello");
    /// ```
    #[inline]
    #[must_use = "use advance if the prefix is not needed"]
    pub fn split_to(&mut self, at: usize) -> Self {
        assert!(self.is_char_boundary(at), "invalid split boundary");

        Self(self.0.split_to(at))
    }

    /// Divides one bytestring into two at an index, returning both parts.
    ///
    /// # Panics
    /// Panics if `mid` is not on a UTF-8 code point boundary, or if it is past the end of the last
    /// code point of the bytestring.
    ///
    /// # Examples
    /// ```
    /// # use bytestring::ByteString;
    /// let text = ByteString::from("hello world");
    /// let (prefix, suffix) = text.split_at(5);
    ///
    /// assert_eq!(prefix, "hello");
    /// assert_eq!(suffix, " world");
    /// assert_eq!(text, "hello world");
    /// ```
    #[doc(alias = "split")]
    #[must_use]
    pub fn split_at(&self, mid: usize) -> (ByteString, ByteString) {
        let this: &str = self.as_ref();
        let _valid_midpoint_check = this.split_at(mid);

        let mut bytes = self.0.clone();
        let first = bytes.split_to(mid);
        let last = bytes;

        unsafe {
            (
                ByteString::from_bytes_unchecked(first),
                ByteString::from_bytes_unchecked(last),
            )
        }
    }

    /// Retains the first `len` bytes. Does nothing if `len` exceeds the length.
    ///
    /// # Panics
    /// Panics if `len` is within the string but is not a UTF-8 code point boundary.
    ///
    /// # Examples
    /// ```
    /// # use bytestring::ByteString;
    /// let mut text = ByteString::from("hello world");
    /// text.truncate(5);
    /// assert_eq!(text, "hello");
    /// ```
    #[inline]
    pub fn truncate(&mut self, len: usize) {
        assert!(
            len >= self.len() || self.is_char_boundary(len),
            "invalid truncate boundary"
        );

        self.0.truncate(len);
    }

    /// Removes the first `cnt` bytes without copying the remaining contents.
    ///
    /// # Panics
    /// Panics if `cnt` is out of bounds or is not a UTF-8 code point boundary.
    ///
    /// # Examples
    /// ```
    /// # use bytestring::ByteString;
    /// let mut text = ByteString::from("µ: value");
    /// text.advance(4);
    /// assert_eq!(text, "value");
    /// ```
    #[inline]
    pub fn advance(&mut self, cnt: usize) {
        assert!(self.is_char_boundary(cnt), "invalid advance boundary");

        bytes::Buf::advance(&mut self.0, cnt);
    }

    /// Unwraps this `ByteString` into the underlying `Bytes` object.
    ///
    /// # Examples
    /// ```
    /// # use bytestring::ByteString;
    /// let text = ByteString::from("hello");
    /// assert_eq!(text.into_bytes(), bytes::Bytes::from_static(b"hello"));
    /// ```
    #[doc(alias = "into_inner")]
    #[doc(alias = "unwrap")]
    #[inline]
    #[must_use]
    pub fn into_bytes(self) -> Bytes {
        self.0
    }
}

impl PartialEq<str> for ByteString {
    #[inline]
    fn eq(&self, other: &str) -> bool {
        &self[..] == other
    }
}

impl<T: AsRef<str>> PartialEq<T> for ByteString {
    #[inline]
    fn eq(&self, other: &T) -> bool {
        &self[..] == other.as_ref()
    }
}

impl AsRef<ByteString> for ByteString {
    #[inline]
    fn as_ref(&self) -> &ByteString {
        self
    }
}

impl AsRef<[u8]> for ByteString {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl AsRef<str> for ByteString {
    #[inline]
    fn as_ref(&self) -> &str {
        self
    }
}

impl hash::Hash for ByteString {
    #[inline]
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
    #[inline]
    fn borrow(&self) -> &str {
        self
    }
}

impl From<ByteString> for Bytes {
    fn from(value: ByteString) -> Self {
        value.into_bytes()
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
        // SAFETY: UTF-8 validity is guaranteed during construction.
        unsafe { String::from_utf8_unchecked(value.0.into()) }
    }
}

impl TryFrom<&[u8]> for ByteString {
    type Error = str::Utf8Error;

    #[inline]
    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        if value.is_ascii() {
            return Ok(ByteString(Bytes::copy_from_slice(value)));
        }

        let _ = str::from_utf8(value)?;
        Ok(ByteString(Bytes::copy_from_slice(value)))
    }
}

impl TryFrom<Vec<u8>> for ByteString {
    type Error = str::Utf8Error;

    #[inline]
    fn try_from(value: Vec<u8>) -> Result<Self, Self::Error> {
        if value.is_ascii() {
            return Ok(ByteString(Bytes::from(value)));
        }

        let buf = String::from_utf8(value).map_err(|err| err.utf8_error())?;
        Ok(ByteString(Bytes::from(buf)))
    }
}

impl TryFrom<Bytes> for ByteString {
    type Error = str::Utf8Error;

    #[inline]
    fn try_from(value: Bytes) -> Result<Self, Self::Error> {
        if value.is_ascii() {
            return Ok(ByteString(value));
        }

        let _ = str::from_utf8(value.as_ref())?;
        Ok(ByteString(value))
    }
}

impl TryFrom<bytes::BytesMut> for ByteString {
    type Error = str::Utf8Error;

    #[inline]
    fn try_from(value: bytes::BytesMut) -> Result<Self, Self::Error> {
        if value.is_ascii() {
            return Ok(ByteString(value.freeze()));
        }

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
    #[inline]
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, fmt)
    }
}

impl fmt::Display for ByteString {
    #[inline]
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&**self, fmt)
    }
}

/// An error from [`ByteString::from_utf8`] that retains the original bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FromUtf8Error {
    bytes: Bytes,
    error: str::Utf8Error,
}

impl FromUtf8Error {
    /// Returns the original bytes without copying them.
    ///
    /// # Examples
    /// ```
    /// # use bytestring::ByteString;
    /// let bytes = bytes::Bytes::from_static(b"a\xff");
    /// let err = ByteString::from_utf8(bytes).unwrap_err();
    /// assert_eq!(err.into_bytes(), bytes::Bytes::from_static(b"a\xff"));
    /// ```
    #[inline]
    #[must_use]
    pub fn into_bytes(self) -> Bytes {
        self.bytes
    }

    /// Returns details of the UTF-8 validation failure.
    ///
    /// # Examples
    /// ```
    /// # use bytestring::ByteString;
    /// let bytes = bytes::Bytes::from_static(b"a\xff");
    /// let err = ByteString::from_utf8(bytes).unwrap_err();
    ///
    /// assert_eq!(err.utf8_error().valid_up_to(), 1);
    /// assert_eq!(err.utf8_error().error_len(), Some(1));
    /// ```
    #[inline]
    #[must_use]
    pub const fn utf8_error(&self) -> str::Utf8Error {
        self.error
    }
}

impl fmt::Display for FromUtf8Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.error, f)
    }
}

impl core::error::Error for FromUtf8Error {}

#[cfg(test)]
mod tests {
    use core::panic::{RefUnwindSafe, UnwindSafe};

    use static_assertions::assert_impl_all;

    use super::*;

    assert_impl_all!(ByteString: Send, Sync, Unpin, Sized);
    assert_impl_all!(ByteString: Clone, Default, Eq, PartialOrd, Ord);
    assert_impl_all!(ByteString: fmt::Debug, fmt::Display);
    assert_impl_all!(ByteString: UnwindSafe, RefUnwindSafe);
}
