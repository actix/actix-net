use std::{
    collections::{vec_deque, VecDeque},
    fmt,
    iter::{FromIterator, FusedIterator},
    net::SocketAddr,
};

/// Connect request
pub trait Address: Unpin + 'static {
    /// Host name of the request
    fn host(&self) -> &str;

    /// Port of the request
    fn port(&self) -> Option<u16>;
}

impl Address for String {
    fn host(&self) -> &str {
        &self
    }

    fn port(&self) -> Option<u16> {
        None
    }
}

impl Address for &'static str {
    fn host(&self) -> &str {
        self
    }

    fn port(&self) -> Option<u16> {
        None
    }
}

/// Connect request
#[derive(Eq, PartialEq, Debug, Hash)]
pub struct Connect<T> {
    pub(crate) req: T,
    pub(crate) port: u16,
    pub(crate) addr: ConnectAddrs,
}

#[derive(Eq, PartialEq, Debug, Hash)]
pub(crate) enum ConnectAddrs {
    One(Option<SocketAddr>),
    Multi(VecDeque<SocketAddr>),
}

impl ConnectAddrs {
    pub(crate) fn is_none(&self) -> bool {
        matches!(*self, Self::One(None))
    }
}

impl Default for ConnectAddrs {
    fn default() -> Self {
        Self::One(None)
    }
}

impl<T: Address> Connect<T> {
    /// Create `Connect` instance by splitting the string by ':' and convert the second part to u16
    pub fn new(req: T) -> Connect<T> {
        let (_, port) = parse(req.host());
        Connect {
            req,
            port: port.unwrap_or(0),
            addr: ConnectAddrs::One(None),
        }
    }

    /// Create new `Connect` instance from host and address. Connector skips name resolution stage
    /// for such connect messages.
    pub fn with(req: T, addr: SocketAddr) -> Connect<T> {
        Connect {
            req,
            port: 0,
            addr: ConnectAddrs::One(Some(addr)),
        }
    }

    /// Use port if address does not provide one.
    ///
    /// By default it set to 0
    pub fn set_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// Use address.
    pub fn set_addr(mut self, addr: Option<SocketAddr>) -> Self {
        self.addr = ConnectAddrs::One(addr);
        self
    }

    /// Use addresses.
    pub fn set_addrs<I>(mut self, addrs: I) -> Self
    where
        I: IntoIterator<Item = SocketAddr>,
    {
        let mut addrs = VecDeque::from_iter(addrs);
        self.addr = if addrs.len() < 2 {
            ConnectAddrs::One(addrs.pop_front())
        } else {
            ConnectAddrs::Multi(addrs)
        };
        self
    }

    /// Host name
    pub fn host(&self) -> &str {
        self.req.host()
    }

    /// Port of the request
    pub fn port(&self) -> u16 {
        self.req.port().unwrap_or(self.port)
    }

    /// Pre-resolved addresses of the request.
    pub fn addrs(&self) -> ConnectAddrsIter<'_> {
        match self.addr {
            ConnectAddrs::One(addr) => ConnectAddrsIter::One(addr),
            ConnectAddrs::Multi(ref addrs) => ConnectAddrsIter::Multi(addrs.iter()),
        }
    }

    /// Takes pre-resolved addresses of the request.
    pub fn take_addrs(&mut self) -> ConnectAddrsIter<'static> {
        match std::mem::take(&mut self.addr) {
            ConnectAddrs::One(addr) => ConnectAddrsIter::One(addr),
            ConnectAddrs::Multi(addrs) => ConnectAddrsIter::MultiOwned(addrs.into_iter()),
        }
    }
}

impl<T: Address> From<T> for Connect<T> {
    fn from(addr: T) -> Self {
        Connect::new(addr)
    }
}

impl<T: Address> fmt::Display for Connect<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.host(), self.port())
    }
}

/// Iterator over addresses in a [`Connect`] request.
#[derive(Clone)]
pub enum ConnectAddrsIter<'a> {
    One(Option<SocketAddr>),
    Multi(vec_deque::Iter<'a, SocketAddr>),
    MultiOwned(vec_deque::IntoIter<SocketAddr>),
}

impl Iterator for ConnectAddrsIter<'_> {
    type Item = SocketAddr;

    fn next(&mut self) -> Option<Self::Item> {
        match *self {
            Self::One(ref mut addr) => addr.take(),
            Self::Multi(ref mut iter) => iter.next().copied(),
            Self::MultiOwned(ref mut iter) => iter.next(),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match *self {
            Self::One(None) => (0, Some(0)),
            Self::One(Some(_)) => (1, Some(1)),
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

impl ExactSizeIterator for ConnectAddrsIter<'_> {}

impl FusedIterator for ConnectAddrsIter<'_> {}

fn parse(host: &str) -> (&str, Option<u16>) {
    let mut parts_iter = host.splitn(2, ':');
    if let Some(host) = parts_iter.next() {
        let port_str = parts_iter.next().unwrap_or("");
        if let Ok(port) = port_str.parse::<u16>() {
            (host, Some(port))
        } else {
            (host, None)
        }
    } else {
        (host, None)
    }
}

pub struct Connection<T, U> {
    io: U,
    req: T,
}

impl<T, U> Connection<T, U> {
    pub fn new(io: U, req: T) -> Self {
        Self { io, req }
    }
}

impl<T, U> Connection<T, U> {
    /// Reconstruct from a parts.
    pub fn from_parts(io: U, req: T) -> Self {
        Self { io, req }
    }

    /// Deconstruct into a parts.
    pub fn into_parts(self) -> (U, T) {
        (self.io, self.req)
    }

    /// Replace inclosed object, return new Stream and old object
    pub fn replace<Y>(self, io: Y) -> (U, Connection<T, Y>) {
        (self.io, Connection { io, req: self.req })
    }

    /// Returns a shared reference to the underlying stream.
    pub fn get_ref(&self) -> &U {
        &self.io
    }

    /// Returns a mutable reference to the underlying stream.
    pub fn get_mut(&mut self) -> &mut U {
        &mut self.io
    }
}

impl<T: Address, U> Connection<T, U> {
    /// Get request
    pub fn host(&self) -> &str {
        &self.req.host()
    }
}

impl<T, U> std::ops::Deref for Connection<T, U> {
    type Target = U;

    fn deref(&self) -> &U {
        &self.io
    }
}

impl<T, U> std::ops::DerefMut for Connection<T, U> {
    fn deref_mut(&mut self) -> &mut U {
        &mut self.io
    }
}

impl<T, U: fmt::Debug> fmt::Debug for Connection<T, U> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Stream {{{:?}}}", self.io)
    }
}
