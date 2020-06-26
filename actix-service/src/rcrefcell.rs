//! Custom cell impl, internal use only
use std::task::{Context, Poll};
use std::{cell::{RefCell, RefMut}, fmt, rc::Rc};

pub(crate) struct RcRefCell<T> {
    inner: Rc<RefCell<T>>,
}

impl<T> Clone for RcRefCell<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T: fmt::Debug> fmt::Debug for RcRefCell<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.inner.fmt(f)
    }
}

impl<T> RcRefCell<T> {
    pub(crate) fn new(inner: T) -> Self {
        Self {
            inner: Rc::new(RefCell::new(inner)),
        }
    }

    pub(crate) fn get_mut(&mut self) -> RefMut<'_, T> {
        self.inner.borrow_mut()
    }
}

impl<T: crate::Service> crate::Service for RcRefCell<T> {
    type Request = T::Request;
    type Response = T::Response;
    type Error = T::Error;
    type Future = T::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.get_mut().poll_ready(cx)
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        self.get_mut().call(req)
    }
}
