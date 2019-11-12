use std::marker::PhantomData;

use super::ServiceFactory;

pub enum MappedConfig<'a, T> {
    Ref(&'a T),
    Owned(T),
}

/// Adapt external config to a config for provided new service
pub fn map_config<T, F, C>(
    factory: T,
    f: F,
) -> impl ServiceFactory<
    Config = C,
    Request = T::Request,
    Response = T::Response,
    Error = T::Error,
    InitError = T::InitError,
>
where
    T: ServiceFactory,
    F: Fn(&C) -> MappedConfig<T::Config>,
{
    MapConfig::new(factory, f)
}

/// Replace config with unit
pub fn unit_config<T, C>(
    new_service: T,
) -> impl ServiceFactory<
    Config = C,
    Request = T::Request,
    Response = T::Response,
    Error = T::Error,
    InitError = T::InitError,
>
where
    T: ServiceFactory<Config = ()>,
{
    UnitConfig::new(new_service)
}

/// `MapInitErr` service combinator
pub(crate) struct MapConfig<A, F, C> {
    a: A,
    f: F,
    e: PhantomData<C>,
}

impl<A, F, C> MapConfig<A, F, C> {
    /// Create new `MapConfig` combinator
    pub fn new(a: A, f: F) -> Self
    where
        A: ServiceFactory,
        F: Fn(&C) -> MappedConfig<A::Config>,
    {
        Self {
            a,
            f,
            e: PhantomData,
        }
    }
}

impl<A, F, C> Clone for MapConfig<A, F, C>
where
    A: Clone,
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

impl<A, F, C> ServiceFactory for MapConfig<A, F, C>
where
    A: ServiceFactory,
    F: Fn(&C) -> MappedConfig<A::Config>,
{
    type Request = A::Request;
    type Response = A::Response;
    type Error = A::Error;

    type Config = C;
    type Service = A::Service;
    type InitError = A::InitError;
    type Future = A::Future;

    fn new_service(&self, cfg: &C) -> Self::Future {
        match (self.f)(cfg) {
            MappedConfig::Ref(cfg) => self.a.new_service(cfg),
            MappedConfig::Owned(cfg) => self.a.new_service(&cfg),
        }
    }
}

/// `MapInitErr` service combinator
pub(crate) struct UnitConfig<A, C> {
    a: A,
    e: PhantomData<C>,
}

impl<A, C> UnitConfig<A, C>
where
    A: ServiceFactory<Config = ()>,
{
    /// Create new `UnitConfig` combinator
    pub(crate) fn new(a: A) -> Self {
        Self { a, e: PhantomData }
    }
}

impl<A, C> Clone for UnitConfig<A, C>
where
    A: Clone,
{
    fn clone(&self) -> Self {
        Self {
            a: self.a.clone(),
            e: PhantomData,
        }
    }
}

impl<A, C> ServiceFactory for UnitConfig<A, C>
where
    A: ServiceFactory<Config = ()>,
{
    type Request = A::Request;
    type Response = A::Response;
    type Error = A::Error;

    type Config = C;
    type Service = A::Service;
    type InitError = A::InitError;
    type Future = A::Future;

    fn new_service(&self, _: &C) -> Self::Future {
        self.a.new_service(&())
    }
}
