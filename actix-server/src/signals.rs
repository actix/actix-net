use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::server::ServerHandle;

/// Types of process signals.
#[allow(dead_code)]
#[derive(PartialEq, Clone, Copy, Debug)]
pub(crate) enum Signal {
    /// `SIGINT`
    Int,

    /// `SIGTERM`
    Term,

    /// `SIGQUIT`
    Quit,
}

/// Process signal listener.
pub(crate) struct Signals {
    srv: ServerHandle,

    #[cfg(not(unix))]
    signals: futures_core::future::LocalBoxFuture<'static, std::io::Result<()>>,

    #[cfg(unix)]
    signals: Vec<(Signal, actix_rt::signal::unix::Signal)>,
}

impl Signals {
    /// Spawns a signal listening future that is able to send commands to the `Server`.
    pub(crate) fn start(srv: ServerHandle) {
        #[cfg(not(unix))]
        {
            actix_rt::spawn(Signals {
                srv,
                signals: Box::pin(actix_rt::signal::ctrl_c()),
            });
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

            actix_rt::spawn(Signals { srv, signals });
        }
    }
}

impl Future for Signals {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        #[cfg(not(unix))]
        match self.signals.as_mut().poll(cx) {
            Poll::Ready(_) => {
                self.srv.signal(Signal::Int);
                Poll::Ready(())
            }
            Poll::Pending => Poll::Pending,
        }

        #[cfg(unix)]
        {
            for (sig, fut) in self.signals.iter_mut() {
                if Pin::new(fut).poll_recv(cx).is_ready() {
                    let sig = *sig;
                    self.srv.signal(sig);
                    return Poll::Ready(());
                }
            }
            Poll::Pending
        }
    }
}
