//! Non-thread-safe channels.
//!
//! See docs for [`mpsc::channel()`].

#![deny(rust_2018_idioms, nonstandard_style)]
#![warn(future_incompatible, missing_docs)]

extern crate alloc;

pub mod mpsc;
