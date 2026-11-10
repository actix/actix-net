//! Serde integration tests for ByteString.

#![cfg(feature = "serde")]

use bytestring::ByteString;
use static_assertions::assert_impl_all;

assert_impl_all!(ByteString: serde_core::Serialize, serde_core::de::DeserializeOwned);

#[test]
fn serialize() {
    let s: ByteString = serde_json::from_str(r#""nice bytes""#).unwrap();
    assert_eq!(s, "nice bytes");
}

#[test]
fn deserialize() {
    let s = serde_json::to_string(&ByteString::from_static("nice bytes")).unwrap();
    assert_eq!(s, r#""nice bytes""#);
}
