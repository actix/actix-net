use derive_more::{Deref, DerefMut};

use super::Host;

/// Wraps underlying I/O and the connection request that initiated it.
#[derive(Debug, Deref, DerefMut)]
pub struct Connection<R, IO> {
    pub(crate) req: R,

    #[deref]
    #[deref_mut]
    pub(crate) io: IO,
}

impl<R, IO> Connection<R, IO> {
    /// Construct new `Connection` from request and IO parts.
    pub(crate) fn new(req: R, io: IO) -> Self {
        Self { req, io }
    }
}

impl<R, IO> Connection<R, IO> {
    /// Deconstructs into IO and request parts.
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

impl<R: Host, IO> Connection<R, IO> {
    /// Returns hostname.
    pub fn hostname(&self) -> &str {
        self.req.hostname()
    }
}
