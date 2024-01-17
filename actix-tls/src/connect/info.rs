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
    Host,
};

/// Connection request information.
///
/// May contain known/pre-resolved socket address(es) or a host that needs resolving with DNS.
#[derive(Debug, PartialEq, Eq, Hash)]
pub struct ConnectInfo<R> {
    pub(crate) request: R,
    pub(crate) port: u16,
    pub(crate) addr: ConnectAddrs,
    pub(crate) local_addr: Option<IpAddr>,
}

impl<R: Host> ConnectInfo<R> {
    /// Constructs new connection info using a request.
    pub fn new(request: R) -> ConnectInfo<R> {
        let port = request.port();

        ConnectInfo {
            request,
            port: port.unwrap_or(0),
            addr: ConnectAddrs::None,
            local_addr: None,
        }
    }

    /// Constructs new connection info from request and known socket address.
    ///
    /// Since socket address is known, [`Connector`](super::Connector) will skip the DNS
    /// resolution step.
    pub fn with_addr(request: R, addr: SocketAddr) -> ConnectInfo<R> {
        ConnectInfo {
            request,
            port: 0,
            addr: ConnectAddrs::One(addr),
            local_addr: None,
        }
    }

    /// Set connection port.
    ///
    /// If request provided a port, this will override it.
    pub fn set_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// Set connection socket address.
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

    /// Set local address to connection with.
    ///
    /// Useful in situations where the IP address bound to a particular network interface is known.
    /// This would make sure the socket is opened through that interface.
    pub fn set_local_addr(mut self, addr: impl Into<IpAddr>) -> Self {
        self.local_addr = Some(addr.into());
        self
    }

    /// Returns a reference to the connection request.
    pub fn request(&self) -> &R {
        &self.request
    }

    /// Returns request hostname.
    pub fn hostname(&self) -> &str {
        self.request.hostname()
    }

    /// Returns request port.
    pub fn port(&self) -> u16 {
        self.request.port().unwrap_or(self.port)
    }

    /// Get borrowed iterator of resolved request addresses.
    ///
    /// # Examples
    /// ```
    /// # use std::net::SocketAddr;
    /// # use actix_tls::connect::ConnectInfo;
    /// let addr = SocketAddr::from(([127, 0, 0, 1], 4242));
    ///
    /// let conn = ConnectInfo::new("localhost");
    /// let mut addrs = conn.addrs();
    /// assert!(addrs.next().is_none());
    ///
    /// let conn = ConnectInfo::with_addr("localhost", addr);
    /// let mut addrs = conn.addrs();
    /// assert_eq!(addrs.next().unwrap(), addr);
    /// ```
    #[allow(clippy::implied_bounds_in_impls)]
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

    /// Take owned iterator resolved request addresses.
    ///
    /// # Examples
    /// ```
    /// # use std::net::SocketAddr;
    /// # use actix_tls::connect::ConnectInfo;
    /// let addr = SocketAddr::from(([127, 0, 0, 1], 4242));
    ///
    /// let mut conn = ConnectInfo::new("localhost");
    /// let mut addrs = conn.take_addrs();
    /// assert!(addrs.next().is_none());
    ///
    /// let mut conn = ConnectInfo::with_addr("localhost", addr);
    /// let mut addrs = conn.take_addrs();
    /// assert_eq!(addrs.next().unwrap(), addr);
    /// ```
    #[allow(clippy::implied_bounds_in_impls)]
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
}

impl<R: Host> From<R> for ConnectInfo<R> {
    fn from(addr: R) -> Self {
        ConnectInfo::new(addr)
    }
}

impl<R: Host> fmt::Display for ConnectInfo<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.hostname(), self.port())
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::*;

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
        let conn = ConnectInfo::new("hello").set_local_addr([127, 0, 0, 1]);
        assert_eq!(
            conn.local_addr.unwrap(),
            IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))
        )
    }

    #[test]
    fn request_ref() {
        let conn = ConnectInfo::new("hello");
        assert_eq!(conn.request(), &"hello")
    }

    #[test]
    fn set_connect_addr_into_option() {
        let addr = SocketAddr::from(([127, 0, 0, 1], 4242));

        let conn = ConnectInfo::new("hello").set_addr(None);
        let mut addrs = conn.addrs();
        assert!(addrs.next().is_none());

        let conn = ConnectInfo::new("hello").set_addr(addr);
        let mut addrs = conn.addrs();
        assert_eq!(addrs.next().unwrap(), addr);

        let conn = ConnectInfo::new("hello").set_addr(Some(addr));
        let mut addrs = conn.addrs();
        assert_eq!(addrs.next().unwrap(), addr);
    }
}
