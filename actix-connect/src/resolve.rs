use std::future::Future;
use std::marker::PhantomData;
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{Context, Poll};

use actix_service::{Service, ServiceFactory};
use futures_util::future::{ready, Ready};
use trust_dns_resolver::TokioAsyncResolver as AsyncResolver;
use trust_dns_resolver::{error::ResolveError, lookup_ip::LookupIp};

use crate::connect::{Address, Connect};
use crate::error::ConnectError;
use crate::get_default_resolver;

/// DNS Resolver Service factory
pub struct ResolverFactory<T> {
    resolver: Option<AsyncResolver>,
    _t: PhantomData<T>,
}

impl<T> ResolverFactory<T> {
    /// Create new resolver instance with custom configuration and options.
    pub fn new(resolver: AsyncResolver) -> Self {
        ResolverFactory {
            resolver: Some(resolver),
            _t: PhantomData,
        }
    }

    pub fn service(&self) -> Resolver<T> {
        Resolver {
            resolver: self.resolver.clone(),
            _t: PhantomData,
        }
    }
}

impl<T> Default for ResolverFactory<T> {
    fn default() -> Self {
        ResolverFactory {
            resolver: None,
            _t: PhantomData,
        }
    }
}

impl<T> Clone for ResolverFactory<T> {
    fn clone(&self) -> Self {
        ResolverFactory {
            resolver: self.resolver.clone(),
            _t: PhantomData,
        }
    }
}

impl<T: Address> ServiceFactory<Connect<T>> for ResolverFactory<T> {
    type Response = Connect<T>;
    type Error = ConnectError;
    type Config = ();
    type Service = Resolver<T>;
    type InitError = ();
    type Future = Ready<Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: ()) -> Self::Future {
        ready(Ok(self.service()))
    }
}

/// DNS Resolver Service
pub struct Resolver<T> {
    resolver: Option<AsyncResolver>,
    _t: PhantomData<T>,
}

impl<T> Resolver<T> {
    /// Create new resolver instance with custom configuration and options.
    pub fn new(resolver: AsyncResolver) -> Self {
        Resolver {
            resolver: Some(resolver),
            _t: PhantomData,
        }
    }
}

impl<T> Default for Resolver<T> {
    fn default() -> Self {
        Resolver {
            resolver: None,
            _t: PhantomData,
        }
    }
}

impl<T> Clone for Resolver<T> {
    fn clone(&self) -> Self {
        Resolver {
            resolver: self.resolver.clone(),
            _t: PhantomData,
        }
    }
}

impl<T: Address> Service<Connect<T>> for Resolver<T> {
    type Response = Connect<T>;
    type Error = ConnectError;
    type Future = ResolverServiceFuture<T>;

    actix_service::always_ready!();

    fn call(&mut self, mut req: Connect<T>) -> Self::Future {
        if req.addr.is_some() {
            ResolverServiceFuture::NoLookUp(Some(req))
        } else if let Ok(ip) = req.host().parse() {
            req.addr = Some(either::Either::Left(SocketAddr::new(ip, req.port())));
            ResolverServiceFuture::NoLookUp(Some(req))
        } else {
            let resolver = self.resolver.as_ref().map(AsyncResolver::clone);
            ResolverServiceFuture::LookUp(Box::pin(async move {
                trace!("DNS resolver: resolving host {:?}", req.host());
                let resolver = if let Some(resolver) = resolver {
                    resolver
                } else {
                    get_default_resolver()
                        .await
                        .expect("Failed to get default resolver")
                };
                ResolverFuture::new(req, &resolver).await
            }))
        }
    }
}

type LocalBoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

#[doc(hidden)]
pub enum ResolverServiceFuture<T: Address> {
    NoLookUp(Option<Connect<T>>),
    LookUp(LocalBoxFuture<'static, Result<Connect<T>, ConnectError>>),
}

impl<T: Address> Future for ResolverServiceFuture<T> {
    type Output = Result<Connect<T>, ConnectError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.get_mut() {
            Self::NoLookUp(conn) => Poll::Ready(Ok(conn.take().unwrap())),
            Self::LookUp(fut) => fut.as_mut().poll(cx),
        }
    }
}

#[doc(hidden)]
/// Resolver future
pub struct ResolverFuture<T: Address> {
    req: Option<Connect<T>>,
    lookup: LocalBoxFuture<'static, Result<LookupIp, ResolveError>>,
}

impl<T: Address> ResolverFuture<T> {
    pub fn new(req: Connect<T>, resolver: &AsyncResolver) -> Self {
        let host = if let Some(host) = req.host().splitn(2, ':').next() {
            host
        } else {
            req.host()
        };

        // Clone data to be moved to the lookup future
        let host_clone = host.to_owned();
        let resolver_clone = resolver.clone();

        ResolverFuture {
            lookup: Box::pin(async move {
                let resolver = resolver_clone;
                resolver.lookup_ip(host_clone).await
            }),
            req: Some(req),
        }
    }
}

impl<T: Address> Future for ResolverFuture<T> {
    type Output = Result<Connect<T>, ConnectError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        match Pin::new(&mut this.lookup).poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Ok(ips)) => {
                let req = this.req.take().unwrap();
                let port = req.port();
                let req = req.set_addrs(ips.iter().map(|ip| SocketAddr::new(ip, port)));

                trace!(
                    "DNS resolver: host {:?} resolved to {:?}",
                    req.host(),
                    req.addrs()
                );

                if req.addr.is_none() {
                    Poll::Ready(Err(ConnectError::NoRecords))
                } else {
                    Poll::Ready(Ok(req))
                }
            }
            Poll::Ready(Err(e)) => {
                trace!(
                    "DNS resolver: failed to resolve host {:?} err: {}",
                    this.req.as_ref().unwrap().host(),
                    e
                );
                Poll::Ready(Err(e.into()))
            }
        }
    }
}
