use std::{
    collections::{vec_deque, VecDeque},
    fmt,
    iter::{self, FromIterator as _},
    mem,
    net::{IpAddr, SocketAddr},
    ops,
};

/// Parse a host into parts (hostname and port).
pub trait Address: Unpin + 'static {
    /// Get hostname part.
    fn hostname(&self) -> &str;

    /// Get optional port part.
    fn port(&self) -> Option<u16> {
        None
    }
}

impl Address for String {
    fn hostname(&self) -> &str {
        self
    }
}

impl Address for &'static str {
    fn hostname(&self) -> &str {
        self
    }
}

#[derive(Debug, Eq, PartialEq, Hash)]
pub(crate) enum ConnectAddrs {
    None,
    One(SocketAddr),
    Multi(VecDeque<SocketAddr>),
}

impl ConnectAddrs {
    pub(crate) fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }

    pub(crate) fn is_some(&self) -> bool {
        !self.is_none()
    }
}

impl Default for ConnectAddrs {
    fn default() -> Self {
        Self::None
    }
}

impl From<Option<SocketAddr>> for ConnectAddrs {
    fn from(addr: Option<SocketAddr>) -> Self {
        match addr {
            Some(addr) => ConnectAddrs::One(addr),
            None => ConnectAddrs::None,
        }
    }
}

/// Connection info.
#[derive(Debug, PartialEq, Eq, Hash)]
pub struct Connect<R> {
    pub(crate) req: R,
    pub(crate) port: u16,
    pub(crate) addr: ConnectAddrs,
    pub(crate) local_addr: Option<IpAddr>,
}

impl<R: Address> Connect<R> {
    /// Create `Connect` instance by splitting the string by ':' and convert the second part to u16
    pub fn new(req: R) -> Connect<R> {
        let (_, port) = parse_host(req.hostname());

        Connect {
            req,
            port: port.unwrap_or(0),
            addr: ConnectAddrs::None,
            local_addr: None,
        }
    }

    /// Create new `Connect` instance from host and address. Connector skips name resolution stage
    /// for such connect messages.
    pub fn with_addr(req: R, addr: SocketAddr) -> Connect<R> {
        Connect {
            req,
            port: 0,
            addr: ConnectAddrs::One(addr),
            local_addr: None,
        }
    }

    /// Use port if address does not provide one.
    ///
    /// Default value is 0.
    pub fn set_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// Set address.
    pub fn set_addr(mut self, addr: Option<SocketAddr>) -> Self {
        self.addr = ConnectAddrs::from(addr);
        self
    }

    /// Set list of addresses.
    pub fn set_addrs<I>(mut self, addrs: I) -> Self
    where
        I: IntoIterator<Item = SocketAddr>,
    {
        let mut addrs = VecDeque::from_iter(addrs);
        self.addr = if addrs.len() < 2 {
            ConnectAddrs::from(addrs.pop_front())
        } else {
            ConnectAddrs::Multi(addrs)
        };
        self
    }

    /// Set local_addr of connect.
    pub fn set_local_addr(mut self, addr: impl Into<IpAddr>) -> Self {
        self.local_addr = Some(addr.into());
        self
    }

    /// Get hostname.
    pub fn hostname(&self) -> &str {
        self.req.hostname()
    }

    /// Get request port.
    pub fn port(&self) -> u16 {
        self.req.port().unwrap_or(self.port)
    }

    /// Get resolved request addresses.
    pub fn addrs(&self) -> ConnectAddrsIter<'_> {
        match self.addr {
            ConnectAddrs::None => ConnectAddrsIter::None,
            ConnectAddrs::One(addr) => ConnectAddrsIter::One(addr),
            ConnectAddrs::Multi(ref addrs) => ConnectAddrsIter::Multi(addrs.iter()),
        }
    }

    /// Take resolved request addresses.
    pub fn take_addrs(&mut self) -> ConnectAddrsIter<'static> {
        match mem::take(&mut self.addr) {
            ConnectAddrs::None => ConnectAddrsIter::None,
            ConnectAddrs::One(addr) => ConnectAddrsIter::One(addr),
            ConnectAddrs::Multi(addrs) => ConnectAddrsIter::MultiOwned(addrs.into_iter()),
        }
    }

    /// Returns a reference to the connection request.
    pub fn request(&self) -> &R {
        &self.req
    }
}

impl<R: Address> From<R> for Connect<R> {
    fn from(addr: R) -> Self {
        Connect::new(addr)
    }
}

impl<R: Address> fmt::Display for Connect<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.hostname(), self.port())
    }
}

/// Iterator over addresses in a [`Connect`] request.
#[derive(Clone)]
pub enum ConnectAddrsIter<'a> {
    None,
    One(SocketAddr),
    Multi(vec_deque::Iter<'a, SocketAddr>),
    MultiOwned(vec_deque::IntoIter<SocketAddr>),
}

impl Iterator for ConnectAddrsIter<'_> {
    type Item = SocketAddr;

    fn next(&mut self) -> Option<Self::Item> {
        match *self {
            Self::None => None,
            Self::One(addr) => {
                *self = Self::None;
                Some(addr)
            }
            Self::Multi(ref mut iter) => iter.next().copied(),
            Self::MultiOwned(ref mut iter) => iter.next(),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match *self {
            Self::None => (0, Some(0)),
            Self::One(_) => (1, Some(1)),
            Self::Multi(ref iter) => iter.size_hint(),
            Self::MultiOwned(ref iter) => iter.size_hint(),
        }
    }
}

impl fmt::Debug for ConnectAddrsIter<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.clone()).finish()
    }
}

impl iter::ExactSizeIterator for ConnectAddrsIter<'_> {}

impl iter::FusedIterator for ConnectAddrsIter<'_> {}

/// Holds underlying I/O and original connection request.
#[derive(Debug)]
pub struct Connection<R, IO> {
    req: R,
    io: IO,
}

impl<R, IO> Connection<R, IO> {
    /// Construct new `Connection` from
    pub fn new(io: IO, req: R) -> Self {
        Self { io, req }
    }
}

impl<R, IO> Connection<R, IO> {
    /// Deconstructs into parts.
    pub fn into_parts(self) -> (IO, R) {
        (self.io, self.req)
    }

    /// Replaces underlying IO, returning old UI and new `Connection`.
    pub fn replace_io<IO2>(self, io: IO2) -> (IO, Connection<R, IO2>) {
        (self.io, Connection { io, req: self.req })
    }

    /// Returns a shared reference to the underlying IO.
    pub fn io_ref(&self) -> &IO {
        &self.io
    }

    /// Returns a mutable reference to the underlying IO.
    pub fn io_mut(&mut self) -> &mut IO {
        &mut self.io
    }

    /// Returns a reference to the connection request.
    pub fn request(&self) -> &R {
        &self.req
    }
}

impl<R: Address, IO> Connection<R, IO> {
    /// Get hostname.
    pub fn host(&self) -> &str {
        self.req.hostname()
    }
}

impl<R, IO> ops::Deref for Connection<R, IO> {
    type Target = IO;

    fn deref(&self) -> &IO {
        &self.io
    }
}

impl<R, IO> ops::DerefMut for Connection<R, IO> {
    fn deref_mut(&mut self) -> &mut IO {
        &mut self.io
    }
}

fn parse_host(host: &str) -> (&str, Option<u16>) {
    let mut parts_iter = host.splitn(2, ':');

    match parts_iter.next() {
        Some(hostname) => {
            let port_str = parts_iter.next().unwrap_or("");
            let port = port_str.parse::<u16>().ok();
            (hostname, port)
        }

        None => (host, None),
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::*;

    #[test]
    fn test_host_parser() {
        assert_eq!(parse_host("example.com"), ("example.com", None));
        assert_eq!(parse_host("example.com:8080"), ("example.com", Some(8080)));
        assert_eq!(parse_host("example:8080"), ("example", Some(8080)));
        assert_eq!(parse_host("example.com:false"), ("example.com", None));
        assert_eq!(parse_host("example.com:false:false"), ("example.com", None));
    }

    #[test]
    fn test_addr_iter_multi() {
        let localhost = SocketAddr::from((IpAddr::from(Ipv4Addr::LOCALHOST), 8080));
        let unspecified = SocketAddr::from((IpAddr::from(Ipv4Addr::UNSPECIFIED), 8080));

        let mut addrs = VecDeque::new();
        addrs.push_back(localhost);
        addrs.push_back(unspecified);

        let mut iter = ConnectAddrsIter::Multi(addrs.iter());
        assert_eq!(iter.next(), Some(localhost));
        assert_eq!(iter.next(), Some(unspecified));
        assert_eq!(iter.next(), None);

        let mut iter = ConnectAddrsIter::MultiOwned(addrs.into_iter());
        assert_eq!(iter.next(), Some(localhost));
        assert_eq!(iter.next(), Some(unspecified));
        assert_eq!(iter.next(), None);
    }

    #[test]
    fn test_addr_iter_single() {
        let localhost = SocketAddr::from((IpAddr::from(Ipv4Addr::LOCALHOST), 8080));

        let mut iter = ConnectAddrsIter::One(localhost);
        assert_eq!(iter.next(), Some(localhost));
        assert_eq!(iter.next(), None);

        let mut iter = ConnectAddrsIter::None;
        assert_eq!(iter.next(), None);
    }

    #[test]
    fn test_local_addr() {
        let conn = Connect::new("hello").set_local_addr([127, 0, 0, 1]);
        assert_eq!(
            conn.local_addr.unwrap(),
            IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))
        )
    }

    #[test]
    fn request_ref() {
        let conn = Connect::new("hello");
        assert_eq!(conn.request(), &"hello")
    }
}
