#![cfg(feature = "connect")]

use std::{
    io,
    net::{Ipv4Addr, SocketAddr},
};

use actix_rt::net::TcpStream;
use actix_server::TestServer;
use actix_service::{fn_service, Service, ServiceFactory};
use actix_tls::connect::{
    ConnectError, ConnectInfo, Connection, Connector, Host, Resolve, Resolver,
};
use futures_core::future::LocalBoxFuture;

#[actix_rt::test]
async fn custom_resolver() {
    /// Always resolves to localhost with the given port.
    struct LocalOnlyResolver;

    impl Resolve for LocalOnlyResolver {
        fn lookup<'a>(
            &'a self,
            _host: &'a str,
            port: u16,
        ) -> LocalBoxFuture<'a, Result<Vec<SocketAddr>, Box<dyn std::error::Error>>> {
            Box::pin(async move {
                let local = format!("127.0.0.1:{}", port).parse().unwrap();
                Ok(vec![local])
            })
        }
    }

    let addr = LocalOnlyResolver.lookup("example.com", 8080).await.unwrap()[0];
    assert_eq!(addr, SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 8080))
}

#[actix_rt::test]
async fn custom_resolver_connect() {
    pub fn connector_factory<T: Host + 'static>(
        resolver: Resolver,
    ) -> impl ServiceFactory<
        ConnectInfo<T>,
        Config = (),
        Response = Connection<T, TcpStream>,
        Error = ConnectError,
        InitError = (),
    > {
        Connector::new(resolver)
    }

    use trust_dns_resolver::TokioAsyncResolver;

    let srv = TestServer::start(|| fn_service(|_io: TcpStream| async { Ok::<_, io::Error>(()) }));

    struct MyResolver {
        trust_dns: TokioAsyncResolver,
    }

    impl Resolve for MyResolver {
        fn lookup<'a>(
            &'a self,
            host: &'a str,
            port: u16,
        ) -> LocalBoxFuture<'a, Result<Vec<SocketAddr>, Box<dyn std::error::Error>>> {
            Box::pin(async move {
                let res = self
                    .trust_dns
                    .lookup_ip(host)
                    .await?
                    .iter()
                    .map(|ip| SocketAddr::new(ip, port))
                    .collect();
                Ok(res)
            })
        }
    }

    let resolver = MyResolver {
        trust_dns: TokioAsyncResolver::tokio_from_system_conf().unwrap(),
    };

    let factory = connector_factory(Resolver::custom(resolver));

    let conn = factory.new_service(()).await.unwrap();
    let con = conn
        .call(ConnectInfo::with_addr("example.com", srv.addr()))
        .await
        .unwrap();
    assert_eq!(con.peer_addr().unwrap(), srv.addr());
}
