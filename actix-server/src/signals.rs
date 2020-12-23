use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::server::Server;
use crate::LocalBoxFuture;

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
    srv: Server,
    #[cfg(not(unix))]
    signals: LocalBoxFuture<'static, std::io::Result<()>>,
    #[cfg(unix)]
    signals: Vec<(Signal, LocalBoxFuture<'static, ()>)>,
}

impl Signals {
    pub(crate) fn start(srv: Server) {
        #[cfg(not(unix))]
        {
            actix_rt::spawn(Signals {
                srv,
                stream: Box::pin(actix_rt::signal::ctrl_c()),
            });
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

            actix_rt::spawn(Signals { srv, signals });
        }
    }
}

impl Future for Signals {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        #[cfg(not(unix))]
        match Pin::new(&mut self.stream).poll(cx) {
            Poll::Ready(_) => {
                self.srv.signal(Signal::Int);
                Poll::Ready(())
            }
            Poll::Pending => Poll::Pending,
        }
        #[cfg(unix)]
        {
            for (sig, fut) in self.signals.iter_mut() {
                if fut.as_mut().poll(cx).is_ready() {
                    let sig = *sig;
                    self.srv.signal(sig);
                    return Poll::Ready(());
                }
            }
            Poll::Pending
        }
    }
}
