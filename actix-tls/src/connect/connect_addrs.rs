use std::{
    collections::{vec_deque, VecDeque},
    fmt, iter,
    net::SocketAddr,
};

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub(crate) enum ConnectAddrs {
    None,
    One(SocketAddr),
    // TODO: consider using smallvec
    Multi(VecDeque<SocketAddr>),
}

impl ConnectAddrs {
    pub(crate) fn is_unresolved(&self) -> bool {
        matches!(self, Self::None)
    }

    pub(crate) fn is_resolved(&self) -> bool {
        !self.is_unresolved()
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

/// Iterator over addresses in a [`Connect`] request.
#[derive(Clone)]
pub(crate) enum ConnectAddrsIter<'a> {
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
