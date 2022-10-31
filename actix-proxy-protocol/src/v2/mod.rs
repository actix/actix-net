use std::{
    io,
    net::{IpAddr, SocketAddr},
};

use tokio::io::{AsyncWrite, AsyncWriteExt as _};

use crate::{AddressFamily, Command, Pp2Crc32c, Tlv, TransportProtocol, Version};

pub(crate) const SIGNATURE: [u8; 12] = [
    0x0D, 0x0A, 0x0D, 0x0A, 0x00, 0x0D, 0x0A, 0x51, 0x55, 0x49, 0x54, 0x0A,
];

#[derive(Debug, Clone)]
pub struct Header {
    command: Command,
    transport_protocol: TransportProtocol,
    address_family: AddressFamily,
    src: SocketAddr,
    dst: SocketAddr,
    tlvs: Vec<Tlv>,
}

impl Header {
    pub const fn new(
        command: Command,
        transport_protocol: TransportProtocol,
        address_family: AddressFamily,
        src: SocketAddr,
        dst: SocketAddr,
        tlvs: Vec<Tlv>,
    ) -> Self {
        Self {
            command,
            transport_protocol,
            address_family,
            src,
            dst,
            tlvs,
        }
    }

    fn v2_len(&self) -> u16 {
        let addr_len = if self.src.is_ipv4() {
            4 + 2 // 4b IPv4 + 2b port number
        } else {
            16 + 2 // 16b IPv6 + 2b port number
        };

        (addr_len * 2) + self.tlvs.iter().map(|tlv| tlv.len()).sum::<u16>()
    }

    pub fn write_to(&self, wrt: &mut impl io::Write) -> io::Result<()> {
        // PROXY v2 signature
        wrt.write_all(&SIGNATURE)?;

        // version | command
        wrt.write_all(&[Version::V2.v2_hi() | self.command.v2_lo()])?;

        // address family | transport protocol
        wrt.write_all(&[self.address_family.v2_hi() | self.transport_protocol.v2_lo()])?;

        // rest-of-header length
        wrt.write_all(&self.v2_len().to_be_bytes())?;

        tracing::debug!("proxy rest-of-header len: {}", self.v2_len());

        fn write_ip_bytes_to(wrt: &mut impl io::Write, ip: IpAddr) -> io::Result<()> {
            match ip {
                IpAddr::V4(ip) => wrt.write_all(&ip.octets()),
                IpAddr::V6(ip) => wrt.write_all(&ip.octets()),
            }
        }

        // L3 (IP) address
        write_ip_bytes_to(wrt, self.src.ip())?;
        write_ip_bytes_to(wrt, self.dst.ip())?;

        // L4 ports
        wrt.write_all(&self.src.port().to_be_bytes())?;
        wrt.write_all(&self.dst.port().to_be_bytes())?;

        // TLVs
        for tlv in &self.tlvs {
            tlv.write_to(wrt)?;
        }

        Ok(())
    }

    pub async fn write_to_tokio(&self, wrt: &mut (impl AsyncWrite + Unpin)) -> io::Result<()> {
        let buf = self.to_vec();
        wrt.write_all(&buf).await
    }

    fn to_vec(&self) -> Vec<u8> {
        // TODO: figure out cap
        let mut buf = Vec::with_capacity(64);
        self.write_to(&mut buf).unwrap();
        buf
    }

    pub fn add_crc23c_checksum_tlv(&mut self) {
        if self.tlvs.iter().any(|tlv| tlv.as_crc32c().is_some()) {
            return;
        }

        // When the checksum is supported by the sender after constructing the header
        // the sender MUST:
        // - initialize the checksum field to '0's.
        // - calculate the CRC32c checksum of the PROXY header as described in RFC4960,
        //   Appendix B [8].
        // - put the resultant value into the checksum field, and leave the rest of
        //   the bits unchanged.

        // add zeroed checksum field to TLVs
        let crc = Pp2Crc32c::default().to_tlv();
        self.tlvs.push(crc);

        // write PROXY header to buffer
        let mut buf = Vec::new();
        self.write_to(&mut buf).unwrap();

        // calculate CRC on buffer and update CRC TLV
        let crc_calc = crc32fast::hash(&buf);
        self.tlvs.last_mut().unwrap().value = crc_calc.to_be_bytes().to_vec();

        tracing::debug!("checksum is {}", crc_calc);
    }

    pub fn validate_crc32c_tlv(&self) -> Option<bool> {
        dbg!(&self.tlvs);

        // exit early if no crc32c TLV is present
        let crc_sent = self.tlvs.iter().filter_map(|tlv| tlv.as_crc32c()).next()?;

        // If the checksum is provided as part of the PROXY header and the checksum
        // functionality is supported by the receiver, the receiver MUST:
        //  - store the received CRC32c checksum value aside.
        //  - replace the 32 bits of the checksum field in the received PROXY header with
        //    all '0's and calculate a CRC32c checksum value of the whole PROXY header.
        //  - verify that the calculated CRC32c checksum is the same as the received
        //    CRC32c checksum. If it is not, the receiver MUST treat the TCP connection
        //    providing the header as invalid.
        // The default procedure for handling an invalid TCP connection is to abort it.

        let mut this = self.clone();
        for tlv in this.tlvs.iter_mut() {
            if tlv.as_crc32c().is_some() {
                tlv.value.fill(0);
            }
        }

        let mut buf = Vec::new();
        this.write_to(&mut buf).unwrap();
        let mut crc_calc = crc32fast::hash(&buf);

        Some(crc_sent.checksum == crc_calc)
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv6Addr;

    use const_str::hex;
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    #[should_panic]
    fn tlv_zero_len() {
        Tlv::new(0x00, vec![]);
    }

    #[test]
    fn write_v2_no_tlvs() {
        let mut exp = Vec::new();
        exp.extend_from_slice(&SIGNATURE); // 0-11
        exp.extend_from_slice(&[0x21, 0x11]); // 12-13
        exp.extend_from_slice(&[0x00, 0x0C]); // 14-15
        exp.extend_from_slice(&[127, 0, 0, 1, 127, 0, 0, 2]); // 16-23
        exp.extend_from_slice(&[0x04, 0xd2, 0x00, 80]); // 24-27

        let header = Header::new(
            Command::Proxy,
            TransportProtocol::Stream,
            AddressFamily::Inet,
            SocketAddr::from(([127, 0, 0, 1], 1234)),
            SocketAddr::from(([127, 0, 0, 2], 80)),
            vec![],
        );

        assert_eq!(header.v2_len(), 12);
        assert_eq!(header.to_vec(), exp);
    }

    #[test]
    fn write_v2_ipv6_tlv_noop() {
        let mut exp = Vec::new();
        exp.extend_from_slice(&SIGNATURE); // 0-11
        exp.extend_from_slice(&[0x20, 0x11]); // 12-13
        exp.extend_from_slice(&[0x00, 0x28]); // 14-15
        exp.extend_from_slice(&hex!("00000000000000000000000000000001")); // 16-31
        exp.extend_from_slice(&hex!("000102030405060708090A0B0C0D0E0F")); // 32-45
        exp.extend_from_slice(&[0x00, 80, 0xff, 0xff]); // 45-49
        exp.extend_from_slice(&[0x04, 0x00, 0x01, 0x00]); // 50-53 NOOP TLV

        let header = Header::new(
            Command::Local,
            TransportProtocol::Stream,
            AddressFamily::Inet,
            SocketAddr::from((Ipv6Addr::LOCALHOST, 80)),
            SocketAddr::from((
                Ipv6Addr::from([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]),
                65535,
            )),
            vec![Tlv::new(0x04, [0])],
        );

        assert_eq!(header.v2_len(), 36 + 4);
        assert_eq!(header.to_vec(), exp);
    }

    #[test]
    fn write_v2_tlv_c2c() {
        let mut exp = Vec::new();
        exp.extend_from_slice(&SIGNATURE); // 0-11
        exp.extend_from_slice(&[0x21, 0x11]); // 12-13
        exp.extend_from_slice(&[0x00, 0x13]); // 14-15
        exp.extend_from_slice(&[127, 0, 0, 1, 127, 0, 0, 1]); // 16-23
        exp.extend_from_slice(&[0x00, 80, 0x00, 80]); // 24-27
        exp.extend_from_slice(&[0x03, 0x00, 0x04, 0x00, 0x00, 0x00, 0x00]); // 28-35 TLV crc32c

        assert_eq!(
            crc32fast::hash(&exp),
            // correct checksum calculated manually
            u32::from_be_bytes([0x08, 0x70, 0x17, 0x7b]),
        );

        // re-assign actual checksum to last 4 bytes of expected byte array
        exp[31..35].copy_from_slice(&[0x08, 0x70, 0x17, 0x7b]);

        let mut header = Header::new(
            Command::Proxy,
            TransportProtocol::Stream,
            AddressFamily::Inet,
            SocketAddr::from(([127, 0, 0, 1], 80)),
            SocketAddr::from(([127, 0, 0, 1], 80)),
            vec![],
        );

        assert!(
            header.validate_crc32c_tlv().is_none(),
            "header doesn't have CRC TLV added yet"
        );

        // add crc32c TLV to header
        header.add_crc23c_checksum_tlv();

        assert_eq!(header.v2_len(), 12 + 7);
        assert_eq!(header.to_vec(), exp);

        // struct can self-validate checksum
        assert_eq!(header.validate_crc32c_tlv().unwrap(), true);

        // mangle crc32c TLV and assert that validate now fails
        *header.tlvs.last_mut().unwrap().value.last_mut().unwrap() = 0x00;
        assert_eq!(header.validate_crc32c_tlv().unwrap(), false);
    }
}
