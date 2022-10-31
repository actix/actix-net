use std::{fmt, io, net::SocketAddr};

use arrayvec::ArrayVec;
use tokio::io::{AsyncWrite, AsyncWriteExt as _};

use crate::AddressFamily;

pub(crate) const SIGNATURE: &str = "PROXY";

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
        write!(wrt, "{}", self)
    }

    pub async fn write_to_tokio(&self, wrt: &mut (impl AsyncWrite + Unpin)) -> io::Result<()> {
        // max length of a V1 header is 107 bytes
        let mut buf = ArrayVec::<_, 107>::new();
        self.write_to(&mut buf)?;
        wrt.write_all(&buf).await
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
