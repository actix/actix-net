use std::{
    fmt,
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use log::trace;

/// Types of process signals.
// #[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(dead_code)] // variants are never constructed on non-unix
pub(crate) enum Signal {
    /// `SIGINT`
    Int,

    /// `SIGTERM`
    Term,

    /// `SIGQUIT`
    Quit,
}

impl fmt::Display for Signal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Signal::Int => "SIGINT",
            Signal::Term => "SIGTERM",
            Signal::Quit => "SIGQUIT",
        })
    }
}

/// Process signal listener.
pub(crate) struct Signals {
    #[cfg(not(unix))]
    signals: futures_core::future::BoxFuture<'static, std::io::Result<()>>,

    #[cfg(unix)]
    signals: Vec<(Signal, actix_rt::signal::unix::Signal)>,
}

impl Signals {
    /// Constructs an OS signal listening future.
    pub(crate) fn new() -> Self {
        trace!("setting up OS signal listener");

        #[cfg(not(unix))]
        {
            Signals {
                signals: Box::pin(actix_rt::signal::ctrl_c()),
            }
        }

        #[cfg(unix)]
        {
            use actix_rt::signal::unix;

            let sig_map = [
                (unix::SignalKind::interrupt(), Signal::Int),
                (unix::SignalKind::terminate(), Signal::Term),
                (unix::SignalKind::quit(), Signal::Quit),
            ];

            let signals = sig_map
                .iter()
                .filter_map(|(kind, sig)| {
                    unix::signal(*kind)
                        .map(|tokio_sig| (*sig, tokio_sig))
                        .map_err(|e| {
                            log::error!(
                                "Can not initialize stream handler for {:?} err: {}",
                                sig,
                                e
                            )
                        })
                        .ok()
                })
                .collect::<Vec<_>>();

            Signals { signals }
        }
    }
}

impl Future for Signals {
    type Output = Signal;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        #[cfg(not(unix))]
        {
            self.signals.as_mut().poll(cx).map(|_| Signal::Int)
        }

        #[cfg(unix)]
        {
            for (sig, fut) in self.signals.iter_mut() {
                // TODO: match on if let Some ?
                if Pin::new(fut).poll_recv(cx).is_ready() {
                    trace!("{} received", sig);
                    return Poll::Ready(*sig);
                }
            }

            Poll::Pending
        }
    }
}
