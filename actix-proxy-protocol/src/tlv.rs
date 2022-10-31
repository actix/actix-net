use std::convert::TryFrom;

pub trait Tlv: Sized {
    const TYPE: u8;

    fn try_from_parts(typ: u8, value: &[u8]) -> Option<Self>;

    fn value_bytes(&self) -> Vec<u8>;
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Crc32c {
    pub(crate) checksum: u32,
}

impl Tlv for Crc32c {
    const TYPE: u8 = 0x03;

    fn try_from_parts(typ: u8, value: &[u8]) -> Option<Self> {
        if typ != Self::TYPE {
            return None;
        }

        let checksum_bytes = <[u8; 4]>::try_from(value).ok()?;

        Some(Self {
            checksum: u32::from_be_bytes(checksum_bytes),
        })
    }

    fn value_bytes(&self) -> Vec<u8> {
        self.checksum.to_be_bytes().to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // #[test]
    // #[should_panic]
    // fn tlv_zero_len() {
    //     Tlv::new(0x00, vec![]);
    // }

    #[test]
    fn tlv_as_crc32c() {
        // noop
        assert_eq!(Crc32c::try_from_parts(0x04, &[0x00]), None);

        assert_eq!(
            Crc32c::try_from_parts(0x03, &[0x08, 0x70, 0x17, 0x7b]),
            Some(Crc32c {
                checksum: 141563771
            })
        );
    }
}
