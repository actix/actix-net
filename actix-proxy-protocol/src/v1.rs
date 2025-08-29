use std::{fmt, io, net::SocketAddr};

use arrayvec::ArrayVec;
use nom::{IResult, Parser as _};
use tokio::io::{AsyncWrite, AsyncWriteExt as _};

use crate::AddressFamily;

pub const SIGNATURE: &str = "PROXY";
pub const MAX_HEADER_SIZE: usize = 107;

#[derive(Debug, Clone)]
pub struct Header {
    /// Address family.
    af: AddressFamily,

    /// Source address.
    src: SocketAddr,

    /// Destination address.
    dst: SocketAddr,
}

impl Header {
    pub const fn new(af: AddressFamily, src: SocketAddr, dst: SocketAddr) -> Self {
        Self { af, src, dst }
    }

    pub const fn new_inet(src: SocketAddr, dst: SocketAddr) -> Self {
        Self::new(AddressFamily::Inet, src, dst)
    }

    pub const fn new_inet6(src: SocketAddr, dst: SocketAddr) -> Self {
        Self::new(AddressFamily::Inet6, src, dst)
    }

    pub fn write_to(&self, wrt: &mut impl io::Write) -> io::Result<()> {
        write!(wrt, "{self}")
    }

    pub async fn write_to_tokio(&self, wrt: &mut (impl AsyncWrite + Unpin)) -> io::Result<()> {
        // max length of a V1 header is 107 bytes
        let mut buf = ArrayVec::<_, MAX_HEADER_SIZE>::new();
        self.write_to(&mut buf)?;
        wrt.write_all(&buf).await
    }

    pub fn try_from_bytes(slice: &[u8]) -> IResult<&[u8], Self> {
        parsing::parse(slice)
    }
}

impl fmt::Display for Header {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{proto_sig} {af} {src_ip} {dst_ip} {src_port} {dst_port}\r\n",
            proto_sig = SIGNATURE,
            af = self.af.v1_str(),
            src_ip = self.src.ip(),
            dst_ip = self.dst.ip(),
            src_port = itoa::Buffer::new().format(self.src.port()),
            dst_port = itoa::Buffer::new().format(self.dst.port()),
        )
    }
}

mod parsing {
    use std::{
        net::{Ipv4Addr, SocketAddrV4},
        str::{self, FromStr},
    };

    use nom::{
        branch::alt,
        bytes::complete::{tag, take_while},
        character::complete::char,
        combinator::{map, map_res},
        IResult,
    };

    use super::*;

    /// Parses a number from serialized representation (as bytes).
    fn parse_number<T: FromStr>(input: &[u8]) -> IResult<&[u8], T> {
        map_res(take_while(|c: u8| c.is_ascii_digit()), |s: &[u8]| {
            let s = str::from_utf8(s).map_err(|_| "utf8 error")?;
            let val = s.parse::<T>().map_err(|_| "u8 parse error")?;
            Ok::<_, Box<dyn std::error::Error>>(val)
        })
        .parse(input)
    }

    /// Parses an address family.
    fn parse_address_family(input: &[u8]) -> IResult<&[u8], AddressFamily> {
        map_res(alt((tag("TCP4"), tag("TCP6"))), |af: &[u8]| match af {
            b"TCP4" => Ok(AddressFamily::Inet),
            b"TCP6" => Ok(AddressFamily::Inet6),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid address family",
            )),
        })
        .parse(input)
    }

    /// Parses an IPv4 address from serialized representation (as bytes).
    fn parse_ipv4(input: &[u8]) -> IResult<&[u8], Ipv4Addr> {
        map(
            (
                parse_number::<u8>,
                char('.'),
                parse_number::<u8>,
                char('.'),
                parse_number::<u8>,
                char('.'),
                parse_number::<u8>,
            ),
            |(a, _, b, _, c, _, d)| Ipv4Addr::new(a, b, c, d),
        )
        .parse(input)
    }

    /// Parses an IPv4 address from ASCII bytes.
    pub(super) fn parse(input: &[u8]) -> IResult<&[u8], Header> {
        map(
            (
                tag(SIGNATURE),
                char(' '),
                parse_address_family,
                char(' '),
                parse_ipv4,
                char(' '),
                parse_ipv4,
                char(' '),
                parse_number::<u16>,
                char(' '),
                parse_number::<u16>,
            ),
            |(_, _, af, _, src_ip, _, dst_ip, _, src_port, _, dst_port)| Header {
                af,
                src: SocketAddr::V4(SocketAddrV4::new(src_ip, src_port)),
                dst: SocketAddr::V4(SocketAddrV4::new(dst_ip, dst_port)),
            },
        )
        .parse(input)
    }
}
