use std::marker::PhantomData;

use futures::future::Future;
use futures::{ready, Poll};

use crate::cell::Cell;
use crate::{IntoFuture, IntoService, NewService, Service};
use std::pin::Pin;
use std::task::Context;

use pin_project::pin_project;

/// Convert `Fn(&Config, &mut Service) -> Future<Service>` fn to a NewService
pub fn apply_cfg<F, C, T, R, S>(
    srv: T,
    f: F,
) -> impl NewService<
    Config = C,
    Request = S::Request,
    Response = S::Response,
    Error = S::Error,
    Service = S,
    InitError = R::Error,
> + Clone
where
    F: FnMut(&C, &mut T) -> R,
    T: Service,
    R: IntoFuture,
    R::Item: IntoService<S>,
    S: Service,
{
    ApplyConfigService {
        f: Cell::new(f),
        srv: Cell::new(srv.into_service()),
        _t: PhantomData,
    }
}

/// Convert `Fn(&Config, &mut Service) -> Future<Service>` fn to a NewService
/// Service get constructor from NewService.
pub fn new_apply_cfg<F, C, T, R, S>(
    srv: T,
    f: F,
) -> impl NewService<
    Config = C,
    Request = S::Request,
    Response = S::Response,
    Error = S::Error,
    Service = S,
    InitError = T::InitError,
> + Clone
where
    C: Clone,
    F: FnMut(&C, &mut T::Service) -> R,
    T: NewService<Config = ()>,
    T::InitError: From<T::Error>,
    R: IntoFuture<Error = T::InitError>,
    R::Item: IntoService<S>,
    S: Service,
{
    ApplyConfigNewService {
        f: Cell::new(f),
        srv: Cell::new(srv),
        _t: PhantomData,
    }
}

/// Convert `Fn(&Config) -> Future<Service>` fn to NewService\
#[pin_project]
struct ApplyConfigService<F, C, T, R, S>
where
    F: FnMut(&C, &mut T) -> R,
    T: Service,
    R: IntoFuture,
    R::Item: IntoService<S>,
    S: Service,
{
    f: Cell<F>,
    #[pin]
    srv: Cell<T>,
    _t: PhantomData<(C, R, S)>,
}

impl<F, C, T, R, S> Clone for ApplyConfigService<F, C, T, R, S>
where
    F: FnMut(&C, &mut T) -> R,
    T: Service,
    R: IntoFuture,
    R::Item: IntoService<S>,
    S: Service,
{
    fn clone(&self) -> Self {
        ApplyConfigService {
            f: self.f.clone(),
            srv: self.srv.clone(),
            _t: PhantomData,
        }
    }
}

impl<F, C, T, R, S> NewService for ApplyConfigService<F, C, T, R, S>
where
    F: FnMut(&C, &mut T) -> R,
    T: Service,
    R: IntoFuture,
    R::Item: IntoService<S>,
    S: Service,
{
    type Config = C;
    type Request = S::Request;
    type Response = S::Response;
    type Error = S::Error;
    type Service = S;

    type InitError = R::Error;
    type Future = FnNewServiceConfigFut<R, S>;

    fn new_service(&self, cfg: &C) -> Self::Future {
        FnNewServiceConfigFut {
            fut: unsafe { (self.f.get_mut_unsafe())(cfg, self.srv.get_mut_unsafe()) }
                .into_future(),
            _t: PhantomData,
        }
    }
}

#[pin_project]
struct FnNewServiceConfigFut<R, S>
where
    R: IntoFuture,
    R::Item: IntoService<S>,
    S: Service,
{
    #[pin]
    fut: R::Future,
    _t: PhantomData<(S,)>,
}

impl<R, S> Future for FnNewServiceConfigFut<R, S>
where
    R: IntoFuture,
    R::Item: IntoService<S>,
    S: Service,
{
    type Output = Result<S, R::Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Ready(Ok(ready!(self.project().fut.poll(cx))?.into_service()))
    }
}

/// Convert `Fn(&Config) -> Future<Service>` fn to NewService
struct ApplyConfigNewService<F, C, T, R, S>
where
    C: Clone,
    F: FnMut(&C, &mut T::Service) -> R,
    T: NewService<Config = ()>,
    R: IntoFuture<Error = T::InitError>,
    R::Item: IntoService<S>,
    S: Service,
{
    f: Cell<F>,
    srv: Cell<T>,
    _t: PhantomData<(C, R, S)>,
}

impl<F, C, T, R, S> Clone for ApplyConfigNewService<F, C, T, R, S>
where
    C: Clone,
    F: FnMut(&C, &mut T::Service) -> R,
    T: NewService<Config = ()>,
    R: IntoFuture<Error = T::InitError>,
    R::Item: IntoService<S>,
    S: Service,
{
    fn clone(&self) -> Self {
        ApplyConfigNewService {
            f: self.f.clone(),
            srv: self.srv.clone(),
            _t: PhantomData,
        }
    }
}

impl<F, C, T, R, S> NewService for ApplyConfigNewService<F, C, T, R, S>
where
    C: Clone,
    F: FnMut(&C, &mut T::Service) -> R,
    T: NewService<Config = ()>,
    T::InitError: From<T::Error>,
    R: IntoFuture<Error = T::InitError>,
    R::Item: IntoService<S>,
    S: Service,
{
    type Config = C;
    type Request = S::Request;
    type Response = S::Response;
    type Error = S::Error;
    type Service = S;

    type InitError = R::Error;
    type Future = ApplyConfigNewServiceFut<F, C, T, R, S>;

    fn new_service(&self, cfg: &C) -> Self::Future {
        ApplyConfigNewServiceFut {
            f: self.f.clone(),
            cfg: cfg.clone(),
            fut: None,
            srv: None,
            srv_fut: Some(self.srv.get_ref().new_service(&())),
            _t: PhantomData,
        }
    }
}

#[pin_project]
struct ApplyConfigNewServiceFut<F, C, T, R, S>
where
    C: Clone,
    F: FnMut(&C, &mut T::Service) -> R,
    T: NewService<Config = ()>,
    T::InitError: From<T::Error>,
    R: IntoFuture<Error = T::InitError>,
    R::Item: IntoService<S>,
    S: Service,
{
    cfg: C,
    f: Cell<F>,
    #[pin]
    srv: Option<T::Service>,
    #[pin]
    srv_fut: Option<T::Future>,
    #[pin]
    fut: Option<R::Future>,
    _t: PhantomData<(S,)>,
}

impl<F, C, T, R, S> Future for ApplyConfigNewServiceFut<F, C, T, R, S>
where
    C: Clone,
    F: FnMut(&C, &mut T::Service) -> R,
    T: NewService<Config = ()>,
    T::InitError: From<T::Error>,
    R: IntoFuture<Error = T::InitError>,
    R::Item: IntoService<S>,
    S: Service,
{
    type Output = Result<S, R::Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();
        'poll: loop {
            if let Some(fut) = this.srv_fut.as_mut().as_pin_mut() {
                match fut.poll(cx)? {
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(srv) => {
                        this.srv_fut.set(None);
                        this.srv.set(Some(srv));
                        continue 'poll;
                    }
                }
            }

            if let Some(fut) = this.fut.as_mut().as_pin_mut() {
                return Poll::Ready(Ok(ready!(fut.poll(cx))?.into_service()));
            } else if let Some(mut srv) = this.srv.as_mut().as_pin_mut() {
                match srv.as_mut().poll_ready(cx)? {
                    Poll::Ready(_) => {
                        this.fut.set(Some(
                            this.f.get_mut()(&this.cfg, unsafe { Pin::get_unchecked_mut(srv) })
                                .into_future(),
                        ));
                        continue 'poll;
                    }
                    Poll::Pending => return Poll::Pending,
                }
            } else {
                return Poll::Pending;
            }
        }
    }
}
