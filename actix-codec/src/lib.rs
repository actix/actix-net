//! Codec utilities for working with framed protocols.
//!
//! Contains adapters to go from streams of bytes, [`AsyncRead`] and [`AsyncWrite`], to framed
//! streams implementing [`Sink`] and [`Stream`]. Framed streams are also known as `transports`.
//!
//! [`Sink`]: futures_sink::Sink
//! [`Stream`]: futures_core::Stream

#![deny(rust_2018_idioms, nonstandard_style)]
#![warn(future_incompatible, missing_docs)]
#![doc(html_logo_url = "https://actix.rs/img/logo.png")]
#![doc(html_favicon_url = "https://actix.rs/favicon.ico")]

pub use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
pub use tokio_util::{
    codec::{Decoder, Encoder},
    io::poll_read_buf,
};

mod bcodec;
mod framed;
mod lines;

pub use self::{
    bcodec::BytesCodec,
    framed::{Framed, FramedParts},
    lines::LinesCodec,
};
