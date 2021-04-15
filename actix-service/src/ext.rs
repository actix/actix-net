use crate::{
    and_then::{AndThenService, AndThenServiceFactory},
    map::Map,
    map_err::MapErr,
    transform_err::TransformMapInitErr,
    IntoService, IntoServiceFactory, Service, ServiceFactory, Transform,
};

/// An extension trait for [`Service`]s that provides a variety of convenient adapters.
pub trait ServiceExt<Req>: Service<Req> {
    /// Map this service's output to a different type, returning a new service
    /// of the resulting type.
    ///
    /// This function is similar to the `Option::map` or `Iterator::map` where
    /// it will change the type of the underlying service.
    ///
    /// Note that this function consumes the receiving service and returns a
    /// wrapped version of it, similar to the existing `map` methods in the
    /// standard library.
    fn map<F, R>(self, f: F) -> Map<Self, F, Req, R>
    where
        Self: Sized,
        F: FnMut(Self::Response) -> R,
    {
        Map::new(self, f)
    }

    /// Map this service's error to a different error, returning a new service.
    ///
    /// This function is similar to the `Result::map_err` where it will change
    /// the error type of the underlying service. For example, this can be useful to
    /// ensure that services have the same error type.
    ///
    /// Note that this function consumes the receiving service and returns a
    /// wrapped version of it.
    fn map_err<F, E>(self, f: F) -> MapErr<Self, Req, F, E>
    where
        Self: Sized,
        F: Fn(Self::Error) -> E,
    {
        MapErr::new(self, f)
    }

    /// Call another service after call to this one has resolved successfully.
    ///
    /// This function can be used to chain two services together and ensure that the second service
    /// isn't called until call to the fist service have finished. Result of the call to the first
    /// service is used as an input parameter for the second service's call.
    ///
    /// Note that this function consumes the receiving service and returns a wrapped version of it.
    fn and_then<I, S1>(self, service: I) -> AndThenService<Self, S1, Req>
    where
        Self: Sized,
        I: IntoService<S1, Self::Response>,
        S1: Service<Self::Response, Error = Self::Error>,
    {
        AndThenService::new(self, service.into_service())
    }
}

impl<S, Req> ServiceExt<Req> for S where S: Service<Req> {}

/// An extension trait for [`ServiceFactory`]s that provides a variety of convenient adapters.
pub trait ServiceFactoryExt<Req>: ServiceFactory<Req> {
    /// Map this service's output to a different type, returning a new service
    /// of the resulting type.
    fn map<F, R>(self, f: F) -> crate::map::MapServiceFactory<Self, F, Req, R>
    where
        Self: Sized,
        F: FnMut(Self::Response) -> R + Clone,
    {
        crate::map::MapServiceFactory::new(self, f)
    }

    /// Map this service's error to a different error, returning a new service.
    fn map_err<F, E>(self, f: F) -> crate::map_err::MapErrServiceFactory<Self, Req, F, E>
    where
        Self: Sized,
        F: Fn(Self::Error) -> E + Clone,
    {
        crate::map_err::MapErrServiceFactory::new(self, f)
    }

    /// Map this factory's init error to a different error, returning a new service.
    fn map_init_err<F, E>(self, f: F) -> crate::map_init_err::MapInitErr<Self, F, Req, E>
    where
        Self: Sized,
        F: Fn(Self::InitError) -> E + Clone,
    {
        crate::map_init_err::MapInitErr::new(self, f)
    }

    /// Call another service after call to this one has resolved successfully.
    fn and_then<I, SF1>(self, factory: I) -> AndThenServiceFactory<Self, SF1, Req>
    where
        Self: Sized,
        Self::Config: Clone,
        I: IntoServiceFactory<SF1, Self::Response>,
        SF1: ServiceFactory<
            Self::Response,
            Config = Self::Config,
            Error = Self::Error,
            InitError = Self::InitError,
        >,
    {
        AndThenServiceFactory::new(self, factory.into_factory())
    }
}

impl<SF, Req> ServiceFactoryExt<Req> for SF where SF: ServiceFactory<Req> {}

/// An extension trait for [`Transform`]s that provides a variety of convenient adapters.
pub trait TransformExt<S, Req>: Transform<S, Req> {
    /// Return a new `Transform` whose init error is mapped to to a different type.
    fn map_init_err<F, E>(self, f: F) -> TransformMapInitErr<Self, S, Req, F, E>
    where
        Self: Sized,
        F: Fn(Self::InitError) -> E + Clone,
    {
        TransformMapInitErr::new(self, f)
    }
}

impl<T, Req> TransformExt<T, Req> for T where T: Transform<T, Req> {}
