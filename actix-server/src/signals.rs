use std::future::Future;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures_core::stream::Stream;

use crate::server::Server;

/// Different types of process signals
#[derive(PartialEq, Clone, Copy, Debug)]
pub(crate) enum Signal {
    /// SIGHUP
    #[cfg_attr(not(unix), allow(dead_code))]
    Hup,
    /// SIGINT
    Int,
    /// SIGTERM
    #[cfg_attr(not(unix), allow(dead_code))]
    Term,
    /// SIGQUIT
    #[cfg_attr(not(unix), allow(dead_code))]
    Quit,
}

pub(crate) struct Signals {
    srv: Server,
    #[cfg(not(unix))]
    stream: actix_rt::signal::CtrlC,
    #[cfg(unix)]
    streams: Vec<(Signal, actix_rt::signal::unix::Signal)>,
}

impl Signals {
    pub(crate) fn start(srv: Server) -> io::Result<()> {
        actix_rt::spawn({
            #[cfg(not(unix))]
            {
                let stream = actix_rt::signal::ctrl_c()?;
                Signals { srv, stream }
            }

            #[cfg(unix)]
            {
                use actix_rt::signal::unix;

                let mut streams = Vec::new();

                let sig_map = [
                    (unix::SignalKind::interrupt(), Signal::Int),
                    (unix::SignalKind::hangup(), Signal::Hup),
                    (unix::SignalKind::terminate(), Signal::Term),
                    (unix::SignalKind::quit(), Signal::Quit),
                ];

                for (kind, sig) in sig_map.iter() {
                    streams.push((*sig, unix::signal(*kind)?));
                }

                Signals { srv, streams }
            }
        });

        Ok(())
    }
}

impl Future for Signals {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        #[cfg(not(unix))]
        loop {
            match Pin::new(&mut self.stream).poll_next(cx) {
                Poll::Ready(Some(_)) => self.srv.signal(Signal::Int),
                Poll::Ready(None) => return Poll::Ready(()),
                Poll::Pending => return Poll::Pending,
            }
        }
        #[cfg(unix)]
        {
            for idx in 0..self.streams.len() {
                loop {
                    match Pin::new(&mut self.streams[idx].1).poll_next(cx) {
                        Poll::Ready(None) => return Poll::Ready(()),
                        Poll::Pending => break,
                        Poll::Ready(Some(_)) => {
                            let sig = self.streams[idx].0;
                            self.srv.signal(sig);
                        }
                    }
                }
            }
            Poll::Pending
        }
    }
}
