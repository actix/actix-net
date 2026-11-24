use alloc::string::String;
use core::fmt;

use serde_core::{
    de::{Deserialize, Deserializer, Visitor},
    ser::{Serialize, Serializer},
};

use super::{ByteString, SmallByteString};

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

impl Serialize for SmallByteString {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for SmallByteString {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct StringVisitor;

        impl Visitor<'_> for StringVisitor {
            type Value = SmallByteString;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a UTF-8 string")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
                Ok(SmallByteString::from(value))
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
                Ok(SmallByteString::from(value))
            }
        }

        deserializer.deserialize_string(StringVisitor)
    }
}

#[cfg(test)]
mod tests {
    use serde_core::{de::DeserializeOwned, Serialize};
    use static_assertions::assert_impl_all;

    use super::*;

    assert_impl_all!(ByteString: Serialize, DeserializeOwned);

    #[test]
    fn small_string_round_trip() {
        let encoded = r#""hello µ""#;
        let text: SmallByteString = serde_json::from_str(encoded).unwrap();
        assert_eq!(text, "hello µ");
        assert_eq!(serde_json::to_string(&text).unwrap(), encoded);
    }
}
