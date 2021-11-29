use actix_rt::net::TcpStream;
use actix_service::{Service, ServiceFactory};

use super::{Address, Connect, ConnectError, ConnectServiceFactory, Connection, Resolver};

/// Create TCP connector service.
pub fn new_connector<R: Address + 'static>(
    resolver: Resolver,
) -> impl Service<Connect<R>, Response = Connection<R, TcpStream>, Error = ConnectError> + Clone
{
    ConnectServiceFactory::new(resolver).service()
}

/// Create TCP connector service factory.
pub fn new_connector_factory<R: Address + 'static>(
    resolver: Resolver,
) -> impl ServiceFactory<
    Connect<R>,
    Config = (),
    Response = Connection<R, TcpStream>,
    Error = ConnectError,
    InitError = (),
> + Clone {
    ConnectServiceFactory::new(resolver)
}

/// Create TCP connector service with default parameters.
pub fn default_connector<R: Address + 'static>(
) -> impl Service<Connect<R>, Response = Connection<R, TcpStream>, Error = ConnectError> + Clone
{
    new_connector(Resolver::Default)
}

/// Create TCP connector service factory with default parameters.
pub fn default_connector_factory<R: Address + 'static>() -> impl ServiceFactory<
    Connect<R>,
    Config = (),
    Response = Connection<R, TcpStream>,
    Error = ConnectError,
    InitError = (),
> + Clone {
    new_connector_factory(Resolver::Default)
}
