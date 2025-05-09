use std::{
    fmt,
    future::Future,
    pin::{pin, Pin},
    task::{Context, Poll},
};

use futures_core::future::BoxFuture;
use tracing::trace;

/// Types of process signals.
// #[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(dead_code)] // variants are never constructed on non-unix
pub(crate) enum SignalKind {
    /// Cancellation token or channel.
    Cancel,

    /// OS `SIGINT`.
    OsInt,

    /// OS `SIGTERM`.
    OsTerm,

    /// OS `SIGQUIT`.
    OsQuit,
}

impl fmt::Display for SignalKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            SignalKind::Cancel => "Cancellation token or channel",
            SignalKind::OsInt => "SIGINT",
            SignalKind::OsTerm => "SIGTERM",
            SignalKind::OsQuit => "SIGQUIT",
        })
    }
}

pub(crate) enum StopSignal {
    /// OS signal handling is configured.
    Os(OsSignals),

    /// Cancellation token or channel.
    Cancel(BoxFuture<'static, ()>),
}

impl Future for StopSignal {
    type Output = SignalKind;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.get_mut() {
            StopSignal::Os(os_signals) => pin!(os_signals).poll(cx),
            StopSignal::Cancel(cancel) => pin!(cancel).poll(cx).map(|()| SignalKind::Cancel),
        }
    }
}

/// Process signal listener.
pub(crate) struct OsSignals {
    #[cfg(not(unix))]
    signals: futures_core::future::BoxFuture<'static, std::io::Result<()>>,

    #[cfg(unix)]
    signals: Vec<(SignalKind, actix_rt::signal::unix::Signal)>,
}

impl OsSignals {
    /// Constructs an OS signal listening future.
    pub(crate) fn new() -> Self {
        trace!("setting up OS signal listener");

        #[cfg(not(unix))]
        {
            OsSignals {
                signals: Box::pin(actix_rt::signal::ctrl_c()),
            }
        }

        #[cfg(unix)]
        {
            use actix_rt::signal::unix;

            let sig_map = [
                (unix::SignalKind::interrupt(), SignalKind::OsInt),
                (unix::SignalKind::terminate(), SignalKind::OsTerm),
                (unix::SignalKind::quit(), SignalKind::OsQuit),
            ];

            let signals = sig_map
                .iter()
                .filter_map(|(kind, sig)| {
                    unix::signal(*kind)
                        .map(|tokio_sig| (*sig, tokio_sig))
                        .map_err(|e| {
                            tracing::error!(
                                "can not initialize stream handler for {:?} err: {}",
                                sig,
                                e
                            )
                        })
                        .ok()
                })
                .collect::<Vec<_>>();

            OsSignals { signals }
        }
    }
}

impl Future for OsSignals {
    type Output = SignalKind;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        #[cfg(not(unix))]
        {
            self.signals.as_mut().poll(cx).map(|_| SignalKind::OsInt)
        }

        #[cfg(unix)]
        {
            for (sig, fut) in self.signals.iter_mut() {
                if fut.poll_recv(cx).is_ready() {
                    trace!("{} received", sig);
                    return Poll::Ready(*sig);
                }
            }

            Poll::Pending
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static_assertions::assert_impl_all!(StopSignal: Send, Unpin);
}
