use std::marker::PhantomData;

use futures::future::{ok, Ready};
use futures::Poll;

use super::{NewService, Service};
use std::pin::Pin;
use std::task::Context;

/// Empty service
#[derive(Clone)]
pub struct Blank<R, E> {
    _t: PhantomData<(R, E)>,
}

impl<R, E> Blank<R, E> {
    pub fn err<E1>(self) -> Blank<R, E1> {
        Blank { _t: PhantomData }
    }
}

impl<R> Blank<R, ()> {
    #[allow(clippy::new_ret_no_self)]
    pub fn new<E>() -> Blank<R, E> {
        Blank { _t: PhantomData }
    }
}

impl<R, E> Default for Blank<R, E> {
    fn default() -> Blank<R, E> {
        Blank { _t: PhantomData }
    }
}

impl<R, E> Service for Blank<R, E> {
    type Request = R;
    type Response = R;
    type Error = E;
    type Future = Ready<Result<R, E>>;

    fn poll_ready(
        self: Pin<&mut Self>,
        _ctx: &mut Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: R) -> Self::Future {
        ok(req)
    }
}

/// Empty service factory
pub struct BlankNewService<R, E1, E2 = ()> {
    _t: PhantomData<(R, E1, E2)>,
}

impl<R, E1, E2> BlankNewService<R, E1, E2> {
    pub fn new() -> BlankNewService<R, E1, E2> {
        BlankNewService { _t: PhantomData }
    }
}

impl<R, E1> BlankNewService<R, E1, ()> {
    pub fn new_unit() -> BlankNewService<R, E1, ()> {
        BlankNewService { _t: PhantomData }
    }
}

impl<R, E1, E2> Default for BlankNewService<R, E1, E2> {
    fn default() -> BlankNewService<R, E1, E2> {
        Self::new()
    }
}

impl<R, E1, E2> NewService for BlankNewService<R, E1, E2> {
    type Request = R;
    type Response = R;
    type Error = E1;

    type Config = ();
    type Service = Blank<R, E1>;
    type InitError = E2;
    type Future = Ready<Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: &()) -> Self::Future {
        ok(Blank::default())
    }
}
