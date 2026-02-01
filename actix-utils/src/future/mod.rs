//! Helpers for constructing futures.

mod either;
mod poll_fn;
mod ready;

#[allow(deprecated)]
pub use self::ready::{err, ok, ready, Ready};
pub use self::{
    either::Either,
    poll_fn::{poll_fn, PollFn},
};
