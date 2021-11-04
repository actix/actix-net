use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use futures_core::future::{BoxFuture, LocalBoxFuture};

// a poor man's join future. joined future is only used when starting/stopping the server.
// pin_project and pinned futures are overkill for this task.
pub(crate) struct JoinAll<T> {
    fut: Vec<JoinFuture<T>>,
}

pub(crate) fn join_all<T>(fut: Vec<impl Future<Output = T> + Send + 'static>) -> JoinAll<T> {
    let fut = fut
        .into_iter()
        .map(|f| JoinFuture::Future(Box::pin(f)))
        .collect();

    JoinAll { fut }
}

enum JoinFuture<T> {
    Future(BoxFuture<'static, T>),
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

pub(crate) fn join_all_local<T>(
    fut: Vec<impl Future<Output = T> + 'static>,
) -> JoinAllLocal<T> {
    let fut = fut
        .into_iter()
        .map(|f| JoinLocalFuture::LocalFuture(Box::pin(f)))
        .collect();

    JoinAllLocal { fut }
}

// a poor man's join future. joined future is only used when starting/stopping the server.
// pin_project and pinned futures are overkill for this task.
pub(crate) struct JoinAllLocal<T> {
    fut: Vec<JoinLocalFuture<T>>,
}

enum JoinLocalFuture<T> {
    LocalFuture(LocalBoxFuture<'static, T>),
    Result(Option<T>),
}

impl<T> Unpin for JoinAllLocal<T> {}

impl<T> Future for JoinAllLocal<T> {
    type Output = Vec<T>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut ready = true;

        let this = self.get_mut();
        for fut in this.fut.iter_mut() {
            if let JoinLocalFuture::LocalFuture(f) = fut {
                match f.as_mut().poll(cx) {
                    Poll::Ready(t) => {
                        *fut = JoinLocalFuture::Result(Some(t));
                    }
                    Poll::Pending => ready = false,
                }
            }
        }

        if ready {
            let mut res = Vec::new();
            for fut in this.fut.iter_mut() {
                if let JoinLocalFuture::Result(f) = fut {
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

    use actix_utils::future::ready;

    #[actix_rt::test]
    async fn test_join_all() {
        let futs = vec![ready(Ok(1)), ready(Err(3)), ready(Ok(9))];
        let mut res = join_all(futs).await.into_iter();
        assert_eq!(Ok(1), res.next().unwrap());
        assert_eq!(Err(3), res.next().unwrap());
        assert_eq!(Ok(9), res.next().unwrap());
    }

    #[actix_rt::test]
    async fn test_join_all_local() {
        let futs = vec![ready(Ok(1)), ready(Err(3)), ready(Ok(9))];
        let mut res = join_all_local(futs).await.into_iter();
        assert_eq!(Ok(1), res.next().unwrap());
        assert_eq!(Err(3), res.next().unwrap());
        assert_eq!(Ok(9), res.next().unwrap());
    }
}
