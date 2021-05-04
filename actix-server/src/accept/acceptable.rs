use std::{fmt, io, marker::PhantomData};

use mio::{Registry, Token};

use crate::waker_queue::WakerQueue;

#[doc(hidden)]
/// Trait define IO source that can be managed by [super::Accept].
pub trait Acceptable: fmt::Debug {
    /// Type accepted from IO source.
    type Connection: Send + 'static;

    fn accept(
        &mut self,
        cx: &mut AcceptContext<'_, Self::Connection>,
    ) -> io::Result<Option<Self::Connection>>;

    /// Register IO source to Acceptor [Registry](mio::Registry).
    /// Self must impl [Source](mio::event::Source) trait.
    fn register(&mut self, registry: &Registry, token: Token) -> io::Result<()>;

    /// Deregister IO source to Acceptor [Registry](mio::Registry).
    /// Self must impl [Source](mio::event::Source) trait.
    fn deregister(&mut self, registry: &Registry) -> io::Result<()>;
}

#[doc(hidden)]
/// Context type of Accept thread. Expose Waker and possible other types to public.
pub struct AcceptContext<'a, C> {
    waker: WakerQueue<C>,
    _p: PhantomData<&'a ()>,
}

impl<'a, C> AcceptContext<'a, C> {
    pub(super) fn new(waker: WakerQueue<C>) -> Self {
        Self {
            waker,
            _p: PhantomData,
        }
    }

    // TODO: Make this public
    pub(super) fn waker(&self) -> &WakerQueue<C> {
        &self.waker
    }
}
