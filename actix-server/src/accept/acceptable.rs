use std::{fmt, io};

use mio::{Registry, Token};

#[doc(hidden)]
/// Trait define IO source that can be managed by [super::Accept].
pub trait Acceptable: fmt::Debug {
    /// Type accepted from IO source.
    type Connection: Send + 'static;

    fn accept(&mut self) -> io::Result<Option<Self::Connection>>;

    /// Register IO source to Acceptor [Registry](mio::Registry).
    /// Self must impl [Source](mio::event::Source) trait.
    fn register(&mut self, registry: &Registry, token: Token) -> io::Result<()>;

    /// Deregister IO source to Acceptor [Registry](mio::Registry).
    /// Self must impl [Source](mio::event::Source) trait.
    fn deregister(&mut self, registry: &Registry) -> io::Result<()>;
}
