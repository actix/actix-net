use std::marker::PhantomData;

use super::{IntoServiceFactory, ServiceFactory};

/// Adapt external config argument to a config for provided service factory
///
/// Note that this function consumes the receiving service factory and returns
/// a wrapped version of it.
pub fn map_config<T, U, F, C, Req>(factory: U, f: F) -> MapConfig<T, Req, F, C>
where
    T: ServiceFactory<Req>,
    U: IntoServiceFactory<T, Req>,
    F: Fn(C) -> T::Config,
{
    MapConfig::new(factory.into_factory(), f)
}

/// Replace config with unit
pub fn unit_config<T, U, C, Req>(factory: U) -> UnitConfig<T, C, Req>
where
    T: ServiceFactory<Req, Config = ()>,
    U: IntoServiceFactory<T, Req>,
{
    UnitConfig::new(factory.into_factory())
}

/// `map_config()` adapter service factory
pub struct MapConfig<A, Req, F, C> {
    a: A,
    f: F,
    e: PhantomData<(C, Req)>,
}

impl<T, F, C, Req> MapConfig<T, F, C, Req> {
    /// Create new `MapConfig` combinator
    pub(crate) fn new(a: T, f: F) -> Self
    where
        T: ServiceFactory<Req>,
        F: Fn(C) -> T::Config,
    {
        Self {
            a,
            f,
            e: PhantomData,
        }
    }
}

impl<T, Req, F, C> Clone for MapConfig<T, Req, F, C>
where
    T: Clone,
    F: Clone,
{
    fn clone(&self) -> Self {
        Self {
            a: self.a.clone(),
            f: self.f.clone(),
            e: PhantomData,
        }
    }
}

impl<T, Req, F, C> ServiceFactory<Req> for MapConfig<T, Req, F, C>
where
    T: ServiceFactory<Req>,
    F: Fn(C) -> T::Config,
{
    type Response = T::Response;
    type Error = T::Error;

    type Config = C;
    type Service = T::Service;
    type InitError = T::InitError;
    type Future = T::Future;

    fn new_service(&self, cfg: C) -> Self::Future {
        self.a.new_service((self.f)(cfg))
    }
}

/// `unit_config()` config combinator
pub struct UnitConfig<T, C, Req> {
    a: T,
    e: PhantomData<(C, Req)>,
}

impl<T, C, Req> UnitConfig<T, C, Req>
where
    T: ServiceFactory<Req, Config = ()>,
{
    /// Create new `UnitConfig` combinator
    pub(crate) fn new(a: T) -> Self {
        Self { a, e: PhantomData }
    }
}

impl<T, C, Req> Clone for UnitConfig<T, C, Req>
where
    T: Clone,
{
    fn clone(&self) -> Self {
        Self {
            a: self.a.clone(),
            e: PhantomData,
        }
    }
}

impl<T, C, Req> ServiceFactory<Req> for UnitConfig<T, C, Req>
where
    T: ServiceFactory<Req, Config = ()>,
{
    type Response = T::Response;
    type Error = T::Error;

    type Config = C;
    type Service = T::Service;
    type InitError = T::InitError;
    type Future = T::Future;

    fn new_service(&self, _: C) -> Self::Future {
        self.a.new_service(())
    }
}
