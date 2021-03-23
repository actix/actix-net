use crate::{
    map::Map, map_err::MapErr, transform_err::TransformMapInitErr, Service, ServiceFactory,
    Transform,
};

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
}

impl<S, Req> ServiceExt<Req> for S where S: Service<Req> {}

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
}

impl<SF, Req> ServiceFactoryExt<Req> for SF where SF: ServiceFactory<Req> {}

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
