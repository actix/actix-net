use alloc::string::String;

use serde_core::{
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
mod tests {
    use serde_core::{de::DeserializeOwned, Serialize};
    use static_assertions::assert_impl_all;

    use super::*;

    assert_impl_all!(ByteString: Serialize, DeserializeOwned);
}
