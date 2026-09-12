//! Optional middleware behavior through the public service API.

use std::{
    future::{poll_fn, Future},
    pin::Pin,
    task::{Context, Poll},
};

use actix_service::{fn_service, Service, Transform};

struct AddOne(bool);

impl<S> Transform<S, u32> for AddOne
where
    S: Service<u32, Response = u32, Error = &'static str> + 'static,
{
    type Response = u32;
    type Error = &'static str;
    type Transform = Wrapped<S>;
    type InitError = &'static str;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Transform, Self::InitError>>>>;

    fn new_transform(&self, service: S) -> Self::Future {
        let fail = self.0;
        Box::pin(async move {
            let mut pending = true;
            poll_fn(move |cx| {
                if std::mem::take(&mut pending) {
                    cx.waker().wake_by_ref();
                    Poll::Pending
                } else {
                    Poll::Ready(())
                }
            })
            .await;
            if fail {
                Err("init")
            } else {
                Ok(Wrapped(service))
            }
        })
    }
}

struct Wrapped<S>(S);

impl<S> Service<u32> for Wrapped<S>
where
    S: Service<u32, Response = u32, Error = &'static str>,
{
    type Response = u32;
    type Error = &'static str;
    type Future = S::Future;

    fn poll_ready(&self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Err("middleware readiness"))
    }

    fn call(&self, req: u32) -> Self::Future {
        self.0.call(req + 1)
    }
}

#[actix_rt::test]
async fn optional_transform() {
    for enabled in [false, true] {
        let service = enabled
            .then_some(AddOne(false))
            .new_transform(fn_service(|req: u32| async move {
                if req > 10 {
                    Err("call")
                } else {
                    Ok(req)
                }
            }))
            .await
            .unwrap();

        assert_eq!(service.call(1).await, Ok(if enabled { 2 } else { 1 }));
        assert_eq!(service.call(11).await, Err("call"));
        assert_eq!(
            poll_fn(|cx| service.poll_ready(cx)).await,
            if enabled {
                Err("middleware readiness")
            } else {
                Ok(())
            }
        );
    }
}

#[actix_rt::test]
async fn optional_transform_init_error() {
    let result = Some(AddOne(true))
        .new_transform(fn_service(|req: u32| async move { Ok(req) }))
        .await;
    assert!(matches!(result, Err("init")));
}

struct PendingService(std::cell::Cell<bool>);

impl Service<u32> for PendingService {
    type Response = u32;
    type Error = &'static str;
    type Future = Pin<Box<dyn Future<Output = Result<u32, Self::Error>>>>;

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        if self.0.replace(false) {
            cx.waker().wake_by_ref();
            Poll::Pending
        } else {
            Poll::Ready(Err("inner readiness"))
        }
    }

    fn call(&self, req: u32) -> Self::Future {
        let mut pending = true;
        Box::pin(poll_fn(move |cx| {
            if std::mem::take(&mut pending) {
                cx.waker().wake_by_ref();
                Poll::Pending
            } else {
                Poll::Ready(Ok(req))
            }
        }))
    }
}

#[actix_rt::test]
async fn optional_transform_pending() {
    for enabled in [false, true] {
        let mut constructed = false;
        let service = enabled
            .then(|| {
                constructed = true;
                AddOne(false)
            })
            .new_transform(PendingService(std::cell::Cell::new(true)))
            .await
            .unwrap();
        assert_eq!(constructed, enabled);
        let mut cx = Context::from_waker(futures_util::task::noop_waker_ref());
        if !enabled {
            assert_eq!(service.poll_ready(&mut cx), Poll::Pending);
            assert_eq!(
                service.poll_ready(&mut cx),
                Poll::Ready(Err("inner readiness"))
            );
        }
        let future = service.call(5);
        let mut future = std::pin::pin!(future);
        assert_eq!(future.as_mut().poll(&mut cx), Poll::Pending);
        assert_eq!(future.await, Ok(if enabled { 6 } else { 5 }));
    }
}
