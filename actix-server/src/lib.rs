//! General purpose TCP server.

#![deny(rust_2018_idioms, nonstandard_style)]
#![doc(html_logo_url = "https://actix.rs/img/logo.png")]
#![doc(html_favicon_url = "https://actix.rs/favicon.ico")]

mod accept;
mod builder;
mod config;
mod server;
mod service;
mod signals;
mod socket;
mod test_server;
mod waker_queue;
mod worker;

pub use self::builder::ServerBuilder;
pub use self::config::{ServiceConfig, ServiceRuntime};
pub use self::server::Server;
pub use self::service::ServiceFactory;
pub use self::test_server::TestServer;

#[doc(hidden)]
pub use self::socket::FromStream;

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

/// Socket ID token
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Token(usize);

impl Default for Token {
    fn default() -> Self {
        Self::new()
    }
}

impl Token {
    fn new() -> Self {
        Self(0)
    }

    pub(crate) fn next(&mut self) -> Token {
        let token = Token(self.0);
        self.0 += 1;
        token
    }
}

/// Start server building process
pub fn new() -> ServerBuilder {
    ServerBuilder::default()
}

// temporary Ready type for std::future::{ready, Ready}; Can be removed when MSRV surpass 1.48
#[doc(hidden)]
pub struct Ready<T>(Option<T>);

pub(crate) fn ready<T>(t: T) -> Ready<T> {
    Ready(Some(t))
}

impl<T> Unpin for Ready<T> {}

impl<T> Future for Ready<T> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Ready(self.get_mut().0.take().unwrap())
    }
}

// a poor man's join future. joined future is only used when starting/stopping the server.
// pin_project and pinned futures are overkill for this task.
pub(crate) struct JoinAll<T> {
    fut: Vec<JoinFuture<T>>,
}

pub(crate) fn join_all<T>(fut: Vec<impl Future<Output = T> + 'static>) -> JoinAll<T> {
    let fut = fut
        .into_iter()
        .map(|f| JoinFuture::Future(Box::pin(f)))
        .collect();

    JoinAll { fut }
}

enum JoinFuture<T> {
    Future(Pin<Box<dyn Future<Output = T>>>),
    Result(Option<T>),
}

impl<T> Unpin for JoinAll<T> {}

impl<T> Future for JoinAll<T> {
    type Output = Vec<T>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut ready = true;

        let this = self.get_mut();
        for fut in this.fut.iter_mut() {
            if let JoinFuture::Future(f) = fut {
                match f.as_mut().poll(cx) {
                    Poll::Ready(t) => {
                        *fut = JoinFuture::Result(Some(t));
                    }
                    Poll::Pending => ready = false,
                }
            }
        }

        if ready {
            let mut res = Vec::new();
            for fut in this.fut.iter_mut() {
                if let JoinFuture::Result(f) = fut {
                    res.push(f.take().unwrap());
                }
            }

            Poll::Ready(res)
        } else {
            Poll::Pending
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[actix_rt::test]
    async fn test_join_all() {
        let futs = vec![ready(Ok(1)), ready(Err(3)), ready(Ok(9))];
        let mut res = join_all(futs).await.into_iter();
        assert_eq!(Ok(1), res.next().unwrap());
        assert_eq!(Err(3), res.next().unwrap());
        assert_eq!(Ok(9), res.next().unwrap());
    }
}
