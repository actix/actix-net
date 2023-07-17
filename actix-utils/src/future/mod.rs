//! Helpers for constructing futures.

mod either;
mod poll_fn;
mod ready;

pub use self::{
    either::Either,
    poll_fn::{poll_fn, PollFn},
    ready::{err, ok, ready, Ready},
};
