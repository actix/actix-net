//! Connection info struct.

use std::{
    collections::VecDeque,
    fmt,
    iter::{self, FromIterator as _},
    mem,
    net::{IpAddr, SocketAddr},
};

use super::{
    connect_addrs::{ConnectAddrs, ConnectAddrsIter},
    Address,
};

/// Connection request information.
///
/// May contain known/pre-resolved socket address(es) or a host that needs resolving with DNS.
#[derive(Debug, PartialEq, Eq, Hash)]
pub struct ConnectionInfo<R> {
    pub(crate) req: R,
    pub(crate) port: u16,
    pub(crate) addr: ConnectAddrs,
    pub(crate) local_addr: Option<IpAddr>,
}

impl<R: Address> ConnectionInfo<R> {
    /// Create `Connect` instance by splitting the host at ':' and convert the second part to u16.
    // TODO: assess usage and find nicer API
    pub fn new(req: R) -> ConnectionInfo<R> {
        let (_, port) = parse_host(req.hostname());

        ConnectionInfo {
            req,
            port: port.unwrap_or(0),
            addr: ConnectAddrs::None,
            local_addr: None,
        }
    }

    /// Create new `Connect` instance from host and socket address.
    ///
    /// Since socket address is known, Connector will skip name resolution stage.
    pub fn with_addr(req: R, addr: SocketAddr) -> ConnectionInfo<R> {
        ConnectionInfo {
            req,
            port: 0,
            addr: ConnectAddrs::One(addr),
            local_addr: None,
        }
    }

    /// Set port if address does not provide one.
    pub fn set_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// Set connect address.
    pub fn set_addr(mut self, addr: impl Into<Option<SocketAddr>>) -> Self {
        self.addr = ConnectAddrs::from(addr.into());
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

    /**
    Get resolved request addresses.

    # Examples
    ```
    # use std::net::SocketAddr;
    # use actix_tls::connect::ConnectionInfo;
    let addr = SocketAddr::from(([127, 0, 0, 1], 4242));

    let conn = ConnectionInfo::with_addr("localhost").set_addr(None);
    let mut addrs = conn.addrs();
    assert!(addrs.next().is_none());
    ```
    */
    pub fn addrs(
        &self,
    ) -> impl Iterator<Item = SocketAddr>
           + ExactSizeIterator
           + iter::FusedIterator
           + Clone
           + fmt::Debug
           + '_ {
        match self.addr {
            ConnectAddrs::None => ConnectAddrsIter::None,
            ConnectAddrs::One(addr) => ConnectAddrsIter::One(addr),
            ConnectAddrs::Multi(ref addrs) => ConnectAddrsIter::Multi(addrs.iter()),
        }
    }

    /**
    Take resolved request addresses.

    # Examples
    ```

    ```
    */
    pub fn take_addrs(
        &mut self,
    ) -> impl Iterator<Item = SocketAddr>
           + ExactSizeIterator
           + iter::FusedIterator
           + Clone
           + fmt::Debug
           + 'static {
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

impl<R: Address> From<R> for ConnectionInfo<R> {
    fn from(addr: R) -> Self {
        ConnectionInfo::new(addr)
    }
}

impl<R: Address> fmt::Display for ConnectionInfo<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.hostname(), self.port())
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
        let conn = ConnectionInfo::new("hello").set_local_addr([127, 0, 0, 1]);
        assert_eq!(
            conn.local_addr.unwrap(),
            IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))
        )
    }

    #[test]
    fn request_ref() {
        let conn = ConnectionInfo::new("hello");
        assert_eq!(conn.request(), &"hello")
    }

    #[test]
    fn set_connect_addr_into_option() {
        let addr = SocketAddr::from(([127, 0, 0, 1], 4242));

        let conn = ConnectionInfo::new("hello").set_addr(None);
        let mut addrs = conn.addrs();
        assert!(addrs.next().is_none());

        let conn = ConnectionInfo::new("hello").set_addr(addr);
        let mut addrs = conn.addrs();
        assert_eq!(addrs.next().unwrap(), addr);

        let conn = ConnectionInfo::new("hello").set_addr(Some(addr));
        let mut addrs = conn.addrs();
        assert_eq!(addrs.next().unwrap(), addr);
    }
}
