use core::marker::PhantomData;

use super::{IntoServiceFactory, ServiceFactory};

/// Adapt external config argument to a config for provided service factory
///
/// Note that this function consumes the receiving service factory and returns
/// a wrapped version of it.
pub fn map_config<I, SF, Req, F, Cfg>(factory: I, f: F) -> MapConfig<SF, Req, F, Cfg>
where
    I: IntoServiceFactory<SF, Req>,
    SF: ServiceFactory<Req>,
    F: Fn(Cfg) -> SF::Config,
{
    MapConfig::new(factory.into_factory(), f)
}

/// Replace config with unit.
pub fn unit_config<I, SF, Cfg, Req>(factory: I) -> UnitConfig<SF, Cfg, Req>
where
    I: IntoServiceFactory<SF, Req>,
    SF: ServiceFactory<Req, Config = ()>,
{
    UnitConfig::new(factory.into_factory())
}

/// `map_config()` adapter service factory
pub struct MapConfig<SF, Req, F, Cfg> {
    factory: SF,
    cfg_mapper: F,
    e: PhantomData<fn(Cfg, Req)>,
}

impl<SF, Req, F, Cfg> MapConfig<SF, Req, F, Cfg> {
    /// Create new `MapConfig` combinator
    pub(crate) fn new(factory: SF, cfg_mapper: F) -> Self
    where
        SF: ServiceFactory<Req>,
        F: Fn(Cfg) -> SF::Config,
    {
        Self {
            factory,
            cfg_mapper,
            e: PhantomData,
        }
    }
}

impl<SF, Req, F, Cfg> Clone for MapConfig<SF, Req, F, Cfg>
where
    SF: Clone,
    F: Clone,
{
    fn clone(&self) -> Self {
        Self {
            factory: self.factory.clone(),
            cfg_mapper: self.cfg_mapper.clone(),
            e: PhantomData,
        }
    }
}

impl<SF, Req, F, Cfg> ServiceFactory<Req> for MapConfig<SF, Req, F, Cfg>
where
    SF: ServiceFactory<Req>,
    F: Fn(Cfg) -> SF::Config,
{
    type Response = SF::Response;
    type Error = SF::Error;

    type Config = Cfg;
    type Service = SF::Service;
    type InitError = SF::InitError;
    type Future = SF::Future;

    fn new_service(&self, cfg: Self::Config) -> Self::Future {
        let mapped_cfg = (self.cfg_mapper)(cfg);
        self.factory.new_service(mapped_cfg)
    }
}

/// `unit_config()` config combinator
pub struct UnitConfig<SF, Cfg, Req> {
    factory: SF,
    _phantom: PhantomData<fn(Cfg, Req)>,
}

impl<SF, Cfg, Req> UnitConfig<SF, Cfg, Req>
where
    SF: ServiceFactory<Req, Config = ()>,
{
    /// Create new `UnitConfig` combinator
    pub(crate) fn new(factory: SF) -> Self {
        Self {
            factory,
            _phantom: PhantomData,
        }
    }
}

impl<SF, Cfg, Req> Clone for UnitConfig<SF, Cfg, Req>
where
    SF: Clone,
{
    fn clone(&self) -> Self {
        Self {
            factory: self.factory.clone(),
            _phantom: PhantomData,
        }
    }
}

impl<SF, Cfg, Req> ServiceFactory<Req> for UnitConfig<SF, Cfg, Req>
where
    SF: ServiceFactory<Req, Config = ()>,
{
    type Response = SF::Response;
    type Error = SF::Error;

    type Config = Cfg;
    type Service = SF::Service;
    type InitError = SF::InitError;
    type Future = SF::Future;

    fn new_service(&self, _: Cfg) -> Self::Future {
        self.factory.new_service(())
    }
}
