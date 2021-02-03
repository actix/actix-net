use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures_core::future::BoxFuture;

/// Different types of process signals
#[allow(dead_code)]
#[derive(PartialEq, Clone, Copy, Debug)]
pub(crate) enum Signal {
    /// SIGHUP
    Hup,
    /// SIGINT
    Int,
    /// SIGTERM
    Term,
    /// SIGQUIT
    Quit,
}

pub(crate) struct Signals {
    #[cfg(not(unix))]
    signals: BoxFuture<'static, std::io::Result<()>>,
    #[cfg(unix)]
    signals: Vec<(Signal, BoxFuture<'static, ()>)>,
}

impl Signals {
    pub(crate) fn new() -> Self {
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
                (unix::SignalKind::hangup(), Signal::Hup),
                (unix::SignalKind::terminate(), Signal::Term),
                (unix::SignalKind::quit(), Signal::Quit),
            ];

            let mut signals = Vec::new();

            for (kind, sig) in sig_map.iter() {
                match unix::signal(*kind) {
                    Ok(mut stream) => {
                        let fut = Box::pin(async move {
                            let _ = stream.recv().await;
                        }) as _;
                        signals.push((*sig, fut));
                    }
                    Err(e) => log::error!(
                        "Can not initialize stream handler for {:?} err: {}",
                        sig,
                        e
                    ),
                }
            }

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
                if fut.as_mut().poll(cx).is_ready() {
                    return Poll::Ready(*sig);
                }
            }
            Poll::Pending
        }
    }
}
