//! Service that applies a timeout to requests.
//!
//! If the response does not complete within the specified timeout, the response
//! will be aborted.

use core::future::Future;
use core::marker::PhantomData;
use core::pin::Pin;
use core::task::{Context, Poll};
use core::{fmt, time};

use actix_rt::time::{sleep, Sleep};
use actix_service::{IntoService, Service, Transform};
use pin_project_lite::pin_project;

/// Applies a timeout to requests.
#[derive(Debug)]
pub struct Timeout<E = ()> {
    timeout: time::Duration,
    _t: PhantomData<E>,
}

/// Timeout error
pub enum TimeoutError<E> {
    /// Service error
    Service(E),
    /// Service call timeout
    Timeout,
}

impl<E> From<E> for TimeoutError<E> {
    fn from(err: E) -> Self {
        TimeoutError::Service(err)
    }
}

impl<E: fmt::Debug> fmt::Debug for TimeoutError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TimeoutError::Service(e) => write!(f, "TimeoutError::Service({:?})", e),
            TimeoutError::Timeout => write!(f, "TimeoutError::Timeout"),
        }
    }
}

impl<E: fmt::Display> fmt::Display for TimeoutError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TimeoutError::Service(e) => e.fmt(f),
            TimeoutError::Timeout => write!(f, "Service call timeout"),
        }
    }
}

impl<E: PartialEq> PartialEq for TimeoutError<E> {
    fn eq(&self, other: &TimeoutError<E>) -> bool {
        match self {
            TimeoutError::Service(e1) => match other {
                TimeoutError::Service(e2) => e1 == e2,
                TimeoutError::Timeout => false,
            },
            TimeoutError::Timeout => matches!(other, TimeoutError::Timeout),
        }
    }
}

impl<E> Timeout<E> {
    pub fn new(timeout: time::Duration) -> Self {
        Timeout {
            timeout,
            _t: PhantomData,
        }
    }
}

impl<E> Clone for Timeout<E> {
    fn clone(&self) -> Self {
        Timeout::new(self.timeout)
    }
}

impl<S, E, Req> Transform<S, Req> for Timeout<E>
where
    S: Service<Req>,
{
    type Response = S::Response;
    type Error = TimeoutError<S::Error>;
    type InitError = E;
    type Transform = TimeoutService<S, Req>;
    type Future = TimeoutFuture<Self::Transform, Self::InitError>;

    fn new_transform(&self, service: S) -> Self::Future {
        let service = TimeoutService {
            service,
            timeout: self.timeout,
            _phantom: PhantomData,
        };

        TimeoutFuture {
            service: Some(service),
            _err: PhantomData,
        }
    }
}

pub struct TimeoutFuture<T, E> {
    service: Option<T>,
    _err: PhantomData<E>,
}

impl<T, E> Unpin for TimeoutFuture<T, E> {}

impl<T, E> Future for TimeoutFuture<T, E> {
    type Output = Result<T, E>;

    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Ready(Ok(self.get_mut().service.take().unwrap()))
    }
}

/// Applies a timeout to requests.
#[derive(Debug, Clone)]
pub struct TimeoutService<S, Req> {
    service: S,
    timeout: time::Duration,
    _phantom: PhantomData<Req>,
}

impl<S, Req> TimeoutService<S, Req>
where
    S: Service<Req>,
{
    pub fn new<U>(timeout: time::Duration, service: U) -> Self
    where
        U: IntoService<S, Req>,
    {
        TimeoutService {
            timeout,
            service: service.into_service(),
            _phantom: PhantomData,
        }
    }
}

impl<S, Req> Service<Req> for TimeoutService<S, Req>
where
    S: Service<Req>,
{
    type Response = S::Response;
    type Error = TimeoutError<S::Error>;
    type Future = TimeoutServiceResponse<S, Req>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx).map_err(TimeoutError::Service)
    }

    fn call(&mut self, request: Req) -> Self::Future {
        TimeoutServiceResponse {
            fut: self.service.call(request),
            sleep: sleep(self.timeout),
        }
    }
}

pin_project! {
    /// `TimeoutService` response future
    #[derive(Debug)]
    pub struct TimeoutServiceResponse<S, Req>
    where
        S: Service<Req>
    {
        #[pin]
        fut: S::Future,
        #[pin]
        sleep: Sleep,
    }
}

impl<S, Req> Future for TimeoutServiceResponse<S, Req>
where
    S: Service<Req>,
{
    type Output = Result<S::Response, TimeoutError<S::Error>>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        // First, try polling the future
        if let Poll::Ready(res) = this.fut.poll(cx) {
            return match res {
                Ok(v) => Poll::Ready(Ok(v)),
                Err(e) => Poll::Ready(Err(TimeoutError::Service(e))),
            };
        }

        // Now check the sleep
        this.sleep.poll(cx).map(|_| Err(TimeoutError::Timeout))
    }
}

#[cfg(test)]
mod tests {
    use std::task::Poll;
    use std::time::Duration;

    use super::*;
    use actix_service::{apply, fn_factory, Service, ServiceFactory};
    use futures_util::future::{ok, FutureExt, LocalBoxFuture};

    struct SleepService(Duration);

    impl Service<()> for SleepService {
        type Response = ();
        type Error = ();
        type Future = LocalBoxFuture<'static, Result<(), ()>>;

        actix_service::always_ready!();

        fn call(&mut self, _: ()) -> Self::Future {
            actix_rt::time::sleep(self.0)
                .then(|_| ok::<_, ()>(()))
                .boxed_local()
        }
    }

    #[actix_rt::test]
    async fn test_success() {
        let resolution = Duration::from_millis(100);
        let wait_time = Duration::from_millis(50);

        let mut timeout = TimeoutService::new(resolution, SleepService(wait_time));
        assert_eq!(timeout.call(()).await, Ok(()));
    }

    #[actix_rt::test]
    async fn test_timeout() {
        let resolution = Duration::from_millis(100);
        let wait_time = Duration::from_millis(500);

        let mut timeout = TimeoutService::new(resolution, SleepService(wait_time));
        assert_eq!(timeout.call(()).await, Err(TimeoutError::Timeout));
    }

    #[actix_rt::test]
    async fn test_timeout_new_service() {
        let resolution = Duration::from_millis(100);
        let wait_time = Duration::from_millis(500);

        let timeout = apply(
            Timeout::new(resolution),
            fn_factory(|| ok::<_, ()>(SleepService(wait_time))),
        );
        let mut srv = timeout.new_service(&()).await.unwrap();

        assert_eq!(srv.call(()).await, Err(TimeoutError::Timeout));
    }
}
