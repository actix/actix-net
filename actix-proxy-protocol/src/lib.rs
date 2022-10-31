//! PROXY protocol.

#![deny(rust_2018_idioms, nonstandard_style)]
#![warn(future_incompatible)]
// #![warn(missing_docs)]
#![allow(unused)]
#![doc(html_logo_url = "https://actix.rs/img/logo.png")]
#![doc(html_favicon_url = "https://actix.rs/favicon.ico")]

use std::{
    convert::TryFrom as _,
    fmt, io,
    net::{IpAddr, SocketAddr},
};

use arrayvec::{ArrayString, ArrayVec};
use tokio::io::{AsyncWrite, AsyncWriteExt as _};

pub mod v1;
pub mod v2;

/// PROXY Protocol Version.
#[derive(Debug, Clone, Copy)]
enum Version {
    /// Human-readable header format (Version 1)
    V1,

    /// Binary header format (Version 2)
    V2,
}

impl Version {
    const fn signature(&self) -> &'static [u8] {
        match self {
            Version::V1 => v1::SIGNATURE.as_bytes(),
            Version::V2 => v2::SIGNATURE.as_slice(),
        }
    }

    const fn v2_hi(&self) -> u8 {
        (match self {
            Version::V1 => panic!("v1 not supported in PROXY v2"),
            Version::V2 => 0x2,
        }) << 4
    }
}

/// Command
///
/// other values are unassigned and must not be emitted by senders. Receivers
/// must drop connections presenting unexpected values here.
#[derive(Debug, Clone, Copy)]
pub enum Command {
    /// \x0 : LOCAL : the connection was established on purpose by the proxy
    /// without being relayed. The connection endpoints are the sender and the
    /// receiver. Such connections exist when the proxy sends health-checks to the
    /// server. The receiver must accept this connection as valid and must use the
    /// real connection endpoints and discard the protocol block including the
    /// family which is ignored.
    Local,

    /// \x1 : PROXY : the connection was established on behalf of another node,
    /// and reflects the original connection endpoints. The receiver must then use
    /// the information provided in the protocol block to get original the address.
    Proxy,
}

impl Command {
    const fn v2_lo(&self) -> u8 {
        match self {
            Command::Local => 0x0,
            Command::Proxy => 0x1,
        }
    }
}

/// Address Family.
///
/// maps to the original socket family without necessarily
/// matching the values internally used by the system.
///
/// other values are unspecified and must not be emitted in version 2 of this
/// protocol and must be rejected as invalid by receivers.
#[derive(Debug, Clone, Copy)]
pub enum AddressFamily {
    /// 0x0 : AF_UNSPEC : the connection is forwarded for an unknown, unspecified
    /// or unsupported protocol. The sender should use this family when sending
    /// LOCAL commands or when dealing with unsupported protocol families. The
    /// receiver is free to accept the connection anyway and use the real endpoint
    /// addresses or to reject it. The receiver should ignore address information.
    Unspecified,

    /// 0x1 : AF_INET : the forwarded connection uses the AF_INET address family
    /// (IPv4). The addresses are exactly 4 bytes each in network byte order,
    /// followed by transport protocol information (typically ports).
    Inet,

    /// 0x2 : AF_INET6 : the forwarded connection uses the AF_INET6 address family
    /// (IPv6). The addresses are exactly 16 bytes each in network byte order,
    /// followed by transport protocol information (typically ports).
    Inet6,

    /// 0x3 : AF_UNIX : the forwarded connection uses the AF_UNIX address family
    /// (UNIX). The addresses are exactly 108 bytes each.
    Unix,
}

impl AddressFamily {
    fn v1_str(&self) -> &'static str {
        match self {
            AddressFamily::Inet => "TCP4",
            AddressFamily::Inet6 => "TCP6",
            af => panic!("{:?} is not supported in PROXY v1", af),
        }
    }

    const fn v2_hi(&self) -> u8 {
        (match self {
            AddressFamily::Unspecified => 0x0,
            AddressFamily::Inet => 0x1,
            AddressFamily::Inet6 => 0x2,
            AddressFamily::Unix => 0x3,
        }) << 4
    }
}

/// Transport Protocol.
///
/// other values are unspecified and must not be emitted in version 2 of this
/// protocol and must be rejected as invalid by receivers.
#[derive(Debug, Clone, Copy)]
pub enum TransportProtocol {
    /// 0x0 : UNSPEC : the connection is forwarded for an unknown, unspecified
    /// or unsupported protocol. The sender should use this family when sending
    /// LOCAL commands or when dealing with unsupported protocol families. The
    /// receiver is free to accept the connection anyway and use the real endpoint
    /// addresses or to reject it. The receiver should ignore address information.
    Unspecified,

    /// 0x1 : STREAM : the forwarded connection uses a SOCK_STREAM protocol (eg:
    /// TCP or UNIX_STREAM). When used with AF_INET/AF_INET6 (TCP), the addresses
    /// are followed by the source and destination ports represented on 2 bytes
    /// each in network byte order.
    Stream,

    /// 0x2 : DGRAM : the forwarded connection uses a SOCK_DGRAM protocol (eg:
    /// UDP or UNIX_DGRAM). When used with AF_INET/AF_INET6 (UDP), the addresses
    /// are followed by the source and destination ports represented on 2 bytes
    /// each in network byte order.
    Datagram,
}

impl TransportProtocol {
    const fn v2_lo(&self) -> u8 {
        match self {
            TransportProtocol::Unspecified => 0x0,
            TransportProtocol::Stream => 0x1,
            TransportProtocol::Datagram => 0x2,
        }
    }
}

#[derive(Debug)]
enum ProxyProtocolHeader {
    V1(v1::Header),
    V2(v2::Header),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pp2Crc32c {
    checksum: u32,
}

impl Pp2Crc32c {
    fn to_tlv(&self) -> Tlv {
        Tlv {
            r#type: 0x03,
            value: self.checksum.to_be_bytes().to_vec(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Tlv {
    r#type: u8,
    value: Vec<u8>,
}

impl Tlv {
    pub fn new(r#type: u8, value: impl Into<Vec<u8>>) -> Self {
        let value = value.into();

        assert!(
            !value.is_empty(),
            "TLV values must have length greater than 1",
        );

        Self { r#type, value }
    }

    fn len(&self) -> u16 {
        // 1b type + 2b len
        // + [len]b value
        1 + 2 + (self.value.len() as u16)
    }

    pub fn write_to(&self, wrt: &mut impl io::Write) -> io::Result<()> {
        wrt.write_all(&[self.r#type])?;
        wrt.write_all(&(self.value.len() as u16).to_be_bytes())?;
        wrt.write_all(&self.value)?;
        Ok(())
    }

    pub fn as_crc32c(&self) -> Option<Pp2Crc32c> {
        if self.r#type != 0x03 {
            return None;
        }

        let checksum_bytes = <[u8; 4]>::try_from(self.value.as_slice()).ok()?;

        Some(Pp2Crc32c {
            checksum: u32::from_be_bytes(checksum_bytes),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tlv_as_crc32c() {
        let noop = Tlv::new(0x04, vec![0x00]);
        assert_eq!(noop.as_crc32c(), None);

        let noop = Tlv::new(0x03, vec![0x08, 0x70, 0x17, 0x7b]);
        assert_eq!(
            noop.as_crc32c(),
            Some(Pp2Crc32c {
                checksum: 141563771
            })
        );
    }
}
