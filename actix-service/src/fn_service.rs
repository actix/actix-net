use core::{future::Future, marker::PhantomData};

use crate::{ok, IntoService, IntoServiceFactory, Ready, Service, ServiceFactory};

/// Create `ServiceFactory` for function that can act as a `Service`
pub fn fn_service<F, Fut, Req, Res, Err, Cfg>(
    f: F,
) -> FnServiceFactory<F, Fut, Req, Res, Err, Cfg>
where
    F: Fn(Req) -> Fut + Clone,
    Fut: Future<Output = Result<Res, Err>>,
{
    FnServiceFactory::new(f)
}

/// Create `ServiceFactory` for function that can produce services
///
/// # Examples
/// ```
/// use std::io;
/// use actix_service::{fn_factory, fn_service, Service, ServiceFactory};
/// use futures_util::future::ok;
///
/// /// Service that divides two usize values.
/// async fn div((x, y): (usize, usize)) -> Result<usize, io::Error> {
///     if y == 0 {
///         Err(io::Error::new(io::ErrorKind::Other, "divide by zero"))
///     } else {
///         Ok(x / y)
///     }
/// }
///
/// #[actix_rt::main]
/// async fn main() -> io::Result<()> {
///     // Create service factory that produces `div` services
///     let factory = fn_factory(|| {
///         ok::<_, io::Error>(fn_service(div))
///     });
///
///     // construct new service
///     let srv = factory.new_service(()).await?;
///
///     // now we can use `div` service
///     let result = srv.call((10, 20)).await?;
///
///     println!("10 / 20 = {}", result);
///
///     Ok(())
/// }
/// ```
pub fn fn_factory<F, Cfg, Srv, Req, Fut, Err>(
    f: F,
) -> FnServiceNoConfig<F, Cfg, Srv, Req, Fut, Err>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<Srv, Err>>,
    Srv: Service<Req>,
{
    FnServiceNoConfig::new(f)
}

/// Create `ServiceFactory` for function that accepts config argument and can produce services
///
/// Any function that has following form `Fn(Config) -> Future<Output = Service>` could act as
/// a `ServiceFactory`.
///
/// # Examples
/// ```
/// use std::io;
/// use actix_service::{fn_factory_with_config, fn_service, Service, ServiceFactory};
/// use futures_util::future::ok;
///
/// #[actix_rt::main]
/// async fn main() -> io::Result<()> {
///     // Create service factory. factory uses config argument for
///     // services it generates.
///     let factory = fn_factory_with_config(|y: usize| {
///         ok::<_, io::Error>(fn_service(move |x: usize| ok::<_, io::Error>(x * y)))
///     });
///
///     // construct new service with config argument
///     let srv = factory.new_service(10).await?;
///
///     let result = srv.call(10).await?;
///     assert_eq!(result, 100);
///
///     println!("10 * 10 = {}", result);
///     Ok(())
/// }
/// ```
pub fn fn_factory_with_config<F, Fut, Cfg, Srv, Req, Err>(
    f: F,
) -> FnServiceConfig<F, Fut, Cfg, Srv, Req, Err>
where
    F: Fn(Cfg) -> Fut,
    Fut: Future<Output = Result<Srv, Err>>,
    Srv: Service<Req>,
{
    FnServiceConfig::new(f)
}

pub struct FnService<F, Fut, Req, Res, Err>
where
    F: FnMut(Req) -> Fut,
    Fut: Future<Output = Result<Res, Err>>,
{
    f: F,
    _t: PhantomData<fn(Req)>,
}

impl<F, Fut, Req, Res, Err> FnService<F, Fut, Req, Res, Err>
where
    F: FnMut(Req) -> Fut,
    Fut: Future<Output = Result<Res, Err>>,
{
    pub(crate) fn new(f: F) -> Self {
        Self { f, _t: PhantomData }
    }
}

impl<F, Fut, Req, Res, Err> Clone for FnService<F, Fut, Req, Res, Err>
where
    F: FnMut(Req) -> Fut + Clone,
    Fut: Future<Output = Result<Res, Err>>,
{
    fn clone(&self) -> Self {
        Self::new(self.f.clone())
    }
}

impl<F, Fut, Req, Res, Err> Service<Req> for FnService<F, Fut, Req, Res, Err>
where
    F: Fn(Req) -> Fut,
    Fut: Future<Output = Result<Res, Err>>,
{
    type Response = Res;
    type Error = Err;
    type Future = Fut;

    crate::always_ready!();

    fn call(&self, req: Req) -> Self::Future {
        (self.f)(req)
    }
}

impl<F, Fut, Req, Res, Err> IntoService<FnService<F, Fut, Req, Res, Err>, Req> for F
where
    F: Fn(Req) -> Fut,
    Fut: Future<Output = Result<Res, Err>>,
{
    fn into_service(self) -> FnService<F, Fut, Req, Res, Err> {
        FnService::new(self)
    }
}

pub struct FnServiceFactory<F, Fut, Req, Res, Err, Cfg>
where
    F: Fn(Req) -> Fut,
    Fut: Future<Output = Result<Res, Err>>,
{
    f: F,
    _t: PhantomData<fn(Req, Cfg)>,
}

impl<F, Fut, Req, Res, Err, Cfg> FnServiceFactory<F, Fut, Req, Res, Err, Cfg>
where
    F: Fn(Req) -> Fut + Clone,
    Fut: Future<Output = Result<Res, Err>>,
{
    fn new(f: F) -> Self {
        FnServiceFactory { f, _t: PhantomData }
    }
}

impl<F, Fut, Req, Res, Err, Cfg> Clone for FnServiceFactory<F, Fut, Req, Res, Err, Cfg>
where
    F: Fn(Req) -> Fut + Clone,
    Fut: Future<Output = Result<Res, Err>>,
{
    fn clone(&self) -> Self {
        Self::new(self.f.clone())
    }
}

impl<F, Fut, Req, Res, Err> Service<Req> for FnServiceFactory<F, Fut, Req, Res, Err, ()>
where
    F: Fn(Req) -> Fut + Clone,
    Fut: Future<Output = Result<Res, Err>>,
{
    type Response = Res;
    type Error = Err;
    type Future = Fut;

    crate::always_ready!();

    fn call(&self, req: Req) -> Self::Future {
        (self.f)(req)
    }
}

impl<F, Fut, Req, Res, Err, Cfg> ServiceFactory<Req>
    for FnServiceFactory<F, Fut, Req, Res, Err, Cfg>
where
    F: Fn(Req) -> Fut + Clone,
    Fut: Future<Output = Result<Res, Err>>,
{
    type Response = Res;
    type Error = Err;

    type Config = Cfg;
    type Service = FnService<F, Fut, Req, Res, Err>;
    type InitError = ();
    type Future = Ready<Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: Cfg) -> Self::Future {
        ok(FnService::new(self.f.clone()))
    }
}

impl<F, Fut, Req, Res, Err, Cfg>
    IntoServiceFactory<FnServiceFactory<F, Fut, Req, Res, Err, Cfg>, Req> for F
where
    F: Fn(Req) -> Fut + Clone,
    Fut: Future<Output = Result<Res, Err>>,
{
    fn into_factory(self) -> FnServiceFactory<F, Fut, Req, Res, Err, Cfg> {
        FnServiceFactory::new(self)
    }
}

/// Convert `Fn(&Config) -> Future<Service>` fn to NewService
pub struct FnServiceConfig<F, Fut, Cfg, Srv, Req, Err>
where
    F: Fn(Cfg) -> Fut,
    Fut: Future<Output = Result<Srv, Err>>,
    Srv: Service<Req>,
{
    f: F,
    _t: PhantomData<fn(Cfg, Req) -> (Fut, Srv, Err)>,
}

impl<F, Fut, Cfg, Srv, Req, Err> FnServiceConfig<F, Fut, Cfg, Srv, Req, Err>
where
    F: Fn(Cfg) -> Fut,
    Fut: Future<Output = Result<Srv, Err>>,
    Srv: Service<Req>,
{
    fn new(f: F) -> Self {
        FnServiceConfig { f, _t: PhantomData }
    }
}

impl<F, Fut, Cfg, Srv, Req, Err> Clone for FnServiceConfig<F, Fut, Cfg, Srv, Req, Err>
where
    F: Fn(Cfg) -> Fut + Clone,
    Fut: Future<Output = Result<Srv, Err>>,
    Srv: Service<Req>,
{
    fn clone(&self) -> Self {
        FnServiceConfig {
            f: self.f.clone(),
            _t: PhantomData,
        }
    }
}

impl<F, Fut, Cfg, Srv, Req, Err> ServiceFactory<Req>
    for FnServiceConfig<F, Fut, Cfg, Srv, Req, Err>
where
    F: Fn(Cfg) -> Fut,
    Fut: Future<Output = Result<Srv, Err>>,
    Srv: Service<Req>,
{
    type Response = Srv::Response;
    type Error = Srv::Error;

    type Config = Cfg;
    type Service = Srv;
    type InitError = Err;
    type Future = Fut;

    fn new_service(&self, cfg: Cfg) -> Self::Future {
        (self.f)(cfg)
    }
}

/// Converter for `Fn() -> Future<Service>` fn
pub struct FnServiceNoConfig<F, Cfg, Srv, Req, Fut, Err>
where
    F: Fn() -> Fut,
    Srv: Service<Req>,
    Fut: Future<Output = Result<Srv, Err>>,
{
    f: F,
    _t: PhantomData<fn(Cfg, Req)>,
}

impl<F, Cfg, Srv, Req, Fut, Err> FnServiceNoConfig<F, Cfg, Srv, Req, Fut, Err>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<Srv, Err>>,
    Srv: Service<Req>,
{
    fn new(f: F) -> Self {
        Self { f, _t: PhantomData }
    }
}

impl<F, Cfg, Srv, Req, Fut, Err> ServiceFactory<Req>
    for FnServiceNoConfig<F, Cfg, Srv, Req, Fut, Err>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<Srv, Err>>,
    Srv: Service<Req>,
{
    type Response = Srv::Response;
    type Error = Srv::Error;
    type Config = Cfg;
    type Service = Srv;
    type InitError = Err;
    type Future = Fut;

    fn new_service(&self, _: Cfg) -> Self::Future {
        (self.f)()
    }
}

impl<F, Cfg, Srv, Req, Fut, Err> Clone for FnServiceNoConfig<F, Cfg, Srv, Req, Fut, Err>
where
    F: Fn() -> Fut + Clone,
    Fut: Future<Output = Result<Srv, Err>>,
    Srv: Service<Req>,
{
    fn clone(&self) -> Self {
        Self::new(self.f.clone())
    }
}

impl<F, Cfg, Srv, Req, Fut, Err>
    IntoServiceFactory<FnServiceNoConfig<F, Cfg, Srv, Req, Fut, Err>, Req> for F
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<Srv, Err>>,
    Srv: Service<Req>,
{
    fn into_factory(self) -> FnServiceNoConfig<F, Cfg, Srv, Req, Fut, Err> {
        FnServiceNoConfig::new(self)
    }
}

#[cfg(test)]
mod tests {
    use core::task::Poll;

    use futures_util::future::lazy;

    use super::*;
    use crate::{ok, Service, ServiceFactory};

    #[actix_rt::test]
    async fn test_fn_service() {
        let new_srv = fn_service(|()| ok::<_, ()>("srv"));

        let srv = new_srv.new_service(()).await.unwrap();
        let res = srv.call(()).await;
        assert_eq!(lazy(|cx| srv.poll_ready(cx)).await, Poll::Ready(Ok(())));
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), "srv");
    }

    #[actix_rt::test]
    async fn test_fn_service_service() {
        let srv = fn_service(|()| ok::<_, ()>("srv"));

        let res = srv.call(()).await;
        assert_eq!(lazy(|cx| srv.poll_ready(cx)).await, Poll::Ready(Ok(())));
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), "srv");
    }

    #[actix_rt::test]
    async fn test_fn_service_with_config() {
        let new_srv = fn_factory_with_config(|cfg: usize| {
            ok::<_, ()>(fn_service(move |()| ok::<_, ()>(("srv", cfg))))
        });

        let srv = new_srv.new_service(1).await.unwrap();
        let res = srv.call(()).await;
        assert_eq!(lazy(|cx| srv.poll_ready(cx)).await, Poll::Ready(Ok(())));
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), ("srv", 1));
    }

    #[actix_rt::test]
    async fn test_auto_impl_send() {
        use crate::{map_config, ServiceExt, ServiceFactoryExt};
        use alloc::rc::Rc;

        let srv_1 = fn_service(|_: Rc<u8>| ok::<_, Rc<u8>>(Rc::new(0u8)));

        let fac_1 = fn_factory_with_config(|_: Rc<u8>| {
            ok::<_, Rc<u8>>(fn_service(|_: Rc<u8>| ok::<_, Rc<u8>>(Rc::new(0u8))))
        });

        let fac_2 = fn_factory(|| {
            ok::<_, Rc<u8>>(fn_service(|_: Rc<u8>| ok::<_, Rc<u8>>(Rc::new(0u8))))
        });

        fn is_send<T: Send + Sync + Clone>(_: &T) {}

        is_send(&fac_1);
        is_send(&map_config(fac_1.clone(), |_: Rc<u8>| Rc::new(0u8)));
        is_send(&fac_1.clone().map_err(|_| Rc::new(0u8)));
        is_send(&fac_1.clone().map(|_| Rc::new(0u8)));
        is_send(&fac_1.clone().map_init_err(|_| Rc::new(0u8)));
        // `and_then` is always !Send
        // is_send(&fac_1.clone().and_then(fac_1.clone()));
        is_send(&fac_1.new_service(Rc::new(0u8)).await.unwrap());

        is_send(&fac_2);
        is_send(&fac_2.new_service(Rc::new(0u8)).await.unwrap());

        is_send(&srv_1);
        is_send(&ServiceExt::map(srv_1.clone(), |_| Rc::new(0u8)));
        is_send(&ServiceExt::map_err(srv_1.clone(), |_| Rc::new(0u8)));
        // `and_then` is always !Send
        // is_send(&ServiceExt::and_then(srv_1.clone(), srv_1.clone()));
    }
}
