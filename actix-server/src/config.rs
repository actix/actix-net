use std::collections::HashMap;
use std::future::Future;
use std::{fmt, io};

use actix_rt::net::TcpStream;
use actix_service::{
    fn_service, IntoServiceFactory as IntoBaseServiceFactory,
    ServiceFactory as BaseServiceFactory,
};
use actix_utils::counter::CounterGuard;
use futures_core::future::LocalBoxFuture;
use log::error;

use crate::builder::bind_addr;
use crate::service::{BoxedServerService, InternalServiceFactory, StreamService};
use crate::socket::{MioStream, MioTcpListener, StdSocketAddr, StdTcpListener, ToSocketAddrs};
use crate::{ready, Token};

pub struct ServiceConfig {
    pub(crate) services: Vec<(String, MioTcpListener)>,
    pub(crate) apply: Option<Box<dyn ServiceRuntimeConfiguration>>,
    pub(crate) threads: usize,
    pub(crate) backlog: u32,
}

impl ServiceConfig {
    pub(super) fn new(threads: usize, backlog: u32) -> ServiceConfig {
        ServiceConfig {
            threads,
            backlog,
            services: Vec::new(),
            apply: None,
        }
    }

    /// Set number of workers to start.
    ///
    /// By default server uses number of available logical cpu as workers
    /// count.
    pub fn workers(&mut self, num: usize) {
        self.threads = num;
    }

    /// Add new service to server
    pub fn bind<U, N: AsRef<str>>(&mut self, name: N, addr: U) -> io::Result<&mut Self>
    where
        U: ToSocketAddrs,
    {
        let sockets = bind_addr(addr, self.backlog)?;

        for lst in sockets {
            self._listen(name.as_ref(), lst);
        }

        Ok(self)
    }

    /// Add new service to server
    pub fn listen<N: AsRef<str>>(&mut self, name: N, lst: StdTcpListener) -> &mut Self {
        self._listen(name, MioTcpListener::from_std(lst))
    }

    /// Register service configuration function. This function get called
    /// during worker runtime configuration. It get executed in worker thread.
    pub fn apply<F>(&mut self, f: F) -> io::Result<()>
    where
        F: Fn(&mut ServiceRuntime) + Send + Clone + 'static,
    {
        self.apply = Some(Box::new(f));
        Ok(())
    }

    fn _listen<N: AsRef<str>>(&mut self, name: N, lst: MioTcpListener) -> &mut Self {
        if self.apply.is_none() {
            self.apply = Some(Box::new(not_configured));
        }
        self.services.push((name.as_ref().to_string(), lst));
        self
    }
}

pub(super) struct ConfiguredService {
    rt: Box<dyn ServiceRuntimeConfiguration>,
    names: HashMap<Token, (String, StdSocketAddr)>,
    topics: HashMap<String, Token>,
    services: Vec<Token>,
}

impl ConfiguredService {
    pub(super) fn new(rt: Box<dyn ServiceRuntimeConfiguration>) -> Self {
        ConfiguredService {
            rt,
            names: HashMap::new(),
            topics: HashMap::new(),
            services: Vec::new(),
        }
    }

    pub(super) fn stream(&mut self, token: Token, name: String, addr: StdSocketAddr) {
        self.names.insert(token, (name.clone(), addr));
        self.topics.insert(name, token);
        self.services.push(token);
    }
}

impl InternalServiceFactory for ConfiguredService {
    fn name(&self, token: Token) -> &str {
        &self.names[&token].0
    }

    fn clone_factory(&self) -> Box<dyn InternalServiceFactory> {
        Box::new(Self {
            rt: self.rt.clone(),
            names: self.names.clone(),
            topics: self.topics.clone(),
            services: self.services.clone(),
        })
    }

    fn create(&self) -> LocalBoxFuture<'static, Result<Vec<(Token, BoxedServerService)>, ()>> {
        // configure services
        let mut rt = ServiceRuntime::new(self.topics.clone());
        self.rt.configure(&mut rt);
        rt.validate();
        let mut names = self.names.clone();
        let tokens = self.services.clone();

        // construct services
        Box::pin(async move {
            let mut services = rt.services;
            // TODO: Proper error handling here
            for f in rt.onstart.into_iter() {
                f.await;
            }
            let mut res = vec![];
            for token in tokens {
                if let Some(srv) = services.remove(&token) {
                    let newserv = srv.new_service(());
                    match newserv.await {
                        Ok(serv) => {
                            res.push((token, serv));
                        }
                        Err(_) => {
                            error!("Can not construct service");
                            return Err(());
                        }
                    }
                } else {
                    let name = names.remove(&token).unwrap().0;
                    res.push((
                        token,
                        Box::new(StreamService::new(fn_service(move |_: TcpStream| {
                            error!("Service {:?} is not configured", name);
                            ready::<Result<_, ()>>(Ok(()))
                        }))),
                    ));
                };
            }
            Ok(res)
        })
    }
}

pub(super) trait ServiceRuntimeConfiguration: Send {
    fn clone(&self) -> Box<dyn ServiceRuntimeConfiguration>;

    fn configure(&self, rt: &mut ServiceRuntime);
}

impl<F> ServiceRuntimeConfiguration for F
where
    F: Fn(&mut ServiceRuntime) + Send + Clone + 'static,
{
    fn clone(&self) -> Box<dyn ServiceRuntimeConfiguration> {
        Box::new(self.clone())
    }

    fn configure(&self, rt: &mut ServiceRuntime) {
        (self)(rt)
    }
}

fn not_configured(_: &mut ServiceRuntime) {
    error!("Service is not configured");
}

pub struct ServiceRuntime {
    names: HashMap<String, Token>,
    services: HashMap<Token, BoxedNewService>,
    onstart: Vec<LocalBoxFuture<'static, ()>>,
}

impl ServiceRuntime {
    fn new(names: HashMap<String, Token>) -> Self {
        ServiceRuntime {
            names,
            services: HashMap::new(),
            onstart: Vec::new(),
        }
    }

    fn validate(&self) {
        for (name, token) in &self.names {
            if !self.services.contains_key(&token) {
                error!("Service {:?} is not configured", name);
            }
        }
    }

    /// Register service.
    ///
    /// Name of the service must be registered during configuration stage with
    /// *ServiceConfig::bind()* or *ServiceConfig::listen()* methods.
    pub fn service<T, F>(&mut self, name: &str, service: F)
    where
        F: IntoBaseServiceFactory<T, TcpStream>,
        T: BaseServiceFactory<TcpStream, Config = ()> + 'static,
        T::Future: 'static,
        T::Service: 'static,
        T::InitError: fmt::Debug,
    {
        // let name = name.to_owned();
        if let Some(token) = self.names.get(name) {
            self.services.insert(
                *token,
                Box::new(ServiceFactory {
                    inner: service.into_factory(),
                }),
            );
        } else {
            panic!("Unknown service: {:?}", name);
        }
    }

    /// Execute future before services initialization.
    pub fn on_start<F>(&mut self, fut: F)
    where
        F: Future<Output = ()> + 'static,
    {
        self.onstart.push(Box::pin(fut))
    }
}

type BoxedNewService = Box<
    dyn BaseServiceFactory<
        (Option<CounterGuard>, MioStream),
        Response = (),
        Error = (),
        InitError = (),
        Config = (),
        Service = BoxedServerService,
        Future = LocalBoxFuture<'static, Result<BoxedServerService, ()>>,
    >,
>;

struct ServiceFactory<T> {
    inner: T,
}

impl<T> BaseServiceFactory<(Option<CounterGuard>, MioStream)> for ServiceFactory<T>
where
    T: BaseServiceFactory<TcpStream, Config = ()>,
    T::Future: 'static,
    T::Service: 'static,
    T::Error: 'static,
    T::InitError: fmt::Debug + 'static,
{
    type Response = ();
    type Error = ();
    type Config = ();
    type Service = BoxedServerService;
    type InitError = ();
    type Future = LocalBoxFuture<'static, Result<BoxedServerService, ()>>;

    fn new_service(&self, _: ()) -> Self::Future {
        let fut = self.inner.new_service(());
        Box::pin(async move {
            match fut.await {
                Ok(s) => Ok(Box::new(StreamService::new(s)) as BoxedServerService),
                Err(e) => {
                    error!("Can not construct service: {:?}", e);
                    Err(())
                }
            }
        })
    }
}
