/// An interface for types where host parts (hostname and port) can be derived.
pub trait Address: Unpin + 'static {
    /// Returns hostname part.
    fn hostname(&self) -> &str;

    /// Returns optional port part.
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
