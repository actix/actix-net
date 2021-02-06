# Changes

## Unreleased - 2021-xx-xx


## 3.0.0-beta.3 - 2021-02-06
* Remove `trust-dns-proto` and `trust-dns-resolver`. [#248]
* Use `std::net::ToSocketAddrs` as simple and basic default resolver. [#248]
* Add `Resolve` trait for custom dns resolver. [#248]
* Add `Resolver::new_custom` function to construct custom resolvers. [#248]
* Export `webpki_roots::TLS_SERVER_ROOTS` in `actix_tls::connect` mod and remove
  the export from `actix_tls::accept` [#248]
* Remove `ConnectTakeAddrsIter`. `Connect::take_addrs` now returns `ConnectAddrsIter<'static>`
  as owned iterator. [#248]
* Rename `Address::{host => hostname}` to more accurately describe which URL segment is returned.
* Update `actix-rt` to `2.0.0`. [#273]

[#248]: https://github.com/actix/actix-net/pull/248
[#273]: https://github.com/actix/actix-net/pull/273


## 3.0.0-beta.2 - 2021-xx-xx
* Depend on stable trust-dns packages. [#204]

[#204]: https://github.com/actix/actix-net/pull/204


## 3.0.0-beta.1 - 2020-12-29
* Move acceptors under `accept` module. [#238]
* Merge `actix-connect` crate under `connect` module. [#238]
* Add feature flags to enable acceptors and/or connectors individually. [#238]

[#238]: https://github.com/actix/actix-net/pull/238


## 2.0.0 - 2020-09-03
* `nativetls::NativeTlsAcceptor` is renamed to `nativetls::Acceptor`.
* Where possible, "SSL" terminology is replaced with "TLS".
    * `SslError` is renamed to `TlsError`.
    * `TlsError::Ssl` enum variant is renamed to `TlsError::Tls`.
    * `max_concurrent_ssl_connect` is renamed to `max_concurrent_tls_connect`.


## 2.0.0-alpha.2 - 2020-08-17
* Update `rustls` dependency to 0.18
* Update `tokio-rustls` dependency to 0.14
* Update `webpki-roots` dependency to 0.20


## [2.0.0-alpha.1] - 2020-03-03
* Update `rustls` dependency to 0.17
* Update `tokio-rustls` dependency to 0.13
* Update `webpki-roots` dependency to 0.19


## [1.0.0] - 2019-12-11
* 1.0.0 release


## [1.0.0-alpha.3] - 2019-12-07
* Migrate to tokio 0.2
* Enable rustls acceptor service
* Enable native-tls acceptor service


## [1.0.0-alpha.1] - 2019-12-02
* Split openssl acceptor from actix-server package
