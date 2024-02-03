# Changes

## Unreleased

## 3.3.0

- Add `rustls-0_22` create feature which excludes any root certificate methods or re-exports.

## 3.2.0

- Support Rustls v0.22.
- Add `{accept, connect}::rustls_0_22` modules.
- Add `rustls-0_21-native-roots` and `rustls-0_20-native-roots` crate features which utilize the `rustls-native-certs` crate to enable a `native_roots_cert_store()` functions in each rustls-based `connect` module.
- Implement `Host` for `http::Uri` (`http` crate version `1`).

## 3.1.1

- Fix `rustls` v0.21 version requirement.

## 3.1.0

- Support Rustls v0.21.
- Add `{accept, connect}::rustls_0_21` modules.
- Add `{accept, connect}::rustls_0_20` alias for `{accept, connect}::rustls` modules.
- Minimum supported Rust version (MSRV) is now 1.65.

## 3.0.4

- Logs emitted now use the `tracing` crate with `log` compatibility. [#451]

[#451]: https://github.com/actix/actix-net/pull/451

## 3.0.3

- No significant changes since `3.0.2`.

## 3.0.2

- Expose `connect::Connection::new`. [#439]

[#439]: https://github.com/actix/actix-net/pull/439

## 3.0.1

- No significant changes since `3.0.0`.

## 3.0.0

- No significant changes since `3.0.0-rc.2`.

## 3.0.0-rc.2

- Re-export `openssl::SslConnectorBuilder` in `connect::openssl::reexports`. [#429]

[#429]: https://github.com/actix/actix-net/pull/429

## 3.0.0-rc.1

### Added

- Derive `Debug` for `connect::Connection`. [#422]
- Implement `Display` for `accept::TlsError`. [#422]
- Implement `Error` for `accept::TlsError` where both types also implement `Error`. [#422]
- Implement `Default` for `connect::Resolver`. [#422]
- Implement `Error` for `connect::ConnectError`. [#422]
- Implement `Default` for `connect::tcp::{TcpConnector, TcpConnectorService}`. [#423]
- Implement `Default` for `connect::ConnectorService`. [#423]

### Changed

- The crate's default features flags no longer include `uri`. [#422]
- Useful re-exports from underlying TLS crates are exposed in a `reexports` modules in all acceptors and connectors.
- Convert `connect::ResolverService` from enum to struct. [#422]
- Make `ConnectAddrsIter` private. [#422]
- Mark `tcp::{TcpConnector, TcpConnectorService}` structs `#[non_exhaustive]`. [#423]
- Rename `accept::native_tls::{NativeTlsAcceptorService => AcceptorService}`. [#422]
- Rename `connect::{Address => Host}` trait. [#422]
- Rename method `connect::Connection::{host => hostname}`. [#422]
- Rename struct `connect::{Connect => ConnectInfo}`. [#422]
- Rename struct `connect::{ConnectService => ConnectorService}`. [#422]
- Rename struct `connect::{ConnectServiceFactory => Connector}`. [#422]
- Rename TLS acceptor service future types and hide from docs. [#422]
- Unbox some service futures types. [#422]
- Inline modules in `connect::tls` to `connect` module. [#422]

### Removed

- Remove `connect::{new_connector, new_connector_factory, default_connector, default_connector_factory}` methods. [#422]
- Remove `connect::native_tls::Connector::service` method. [#422]
- Remove redundant `connect::Connection::from_parts` method. [#422]

[#422]: https://github.com/actix/actix-net/pull/422
[#423]: https://github.com/actix/actix-net/pull/423

## 3.0.0-beta.9

- Add configurable timeout for accepting TLS connection. [#393]
- Added `TlsError::Timeout` variant. [#393]
- All TLS acceptor services now use `TlsError` for their error types. [#393]
- Added `TlsError::into_service_error`. [#420]

[#393]: https://github.com/actix/actix-net/pull/393
[#420]: https://github.com/actix/actix-net/pull/420

## 3.0.0-beta.8

- Add `Connect::request` for getting a reference to the connection request. [#415]

[#415]: https://github.com/actix/actix-net/pull/415

## 3.0.0-beta.7

- Add `webpki_roots_cert_store()` to get rustls compatible webpki roots cert store. [#401]
- Alias `connect::ssl` to `connect::tls`. [#401]

[#401]: https://github.com/actix/actix-net/pull/401

## 3.0.0-beta.6

- Update `tokio-rustls` to `0.23` which uses `rustls` `0.20`. [#396]
- Removed a re-export of `Session` from `rustls` as it no longer exist. [#396]
- Minimum supported Rust version (MSRV) is now 1.52.

[#396]: https://github.com/actix/actix-net/pull/396

## 3.0.0-beta.5

- Changed `connect::ssl::rustls::RustlsConnectorService` to return error when `DNSNameRef` generation failed instead of panic. [#296]
- Remove `connect::ssl::openssl::OpensslConnectServiceFactory`. [#297]
- Remove `connect::ssl::openssl::OpensslConnectService`. [#297]
- Add `connect::ssl::native_tls` module for native tls support. [#295]
- Rename `accept::{nativetls => native_tls}`. [#295]
- Remove `connect::TcpConnectService` type. Service caller expecting a `TcpStream` should use `connect::ConnectService` instead and call `Connection<T, TcpStream>::into_parts`. [#299]

[#295]: https://github.com/actix/actix-net/pull/295
[#296]: https://github.com/actix/actix-net/pull/296
[#297]: https://github.com/actix/actix-net/pull/297
[#299]: https://github.com/actix/actix-net/pull/299

## 3.0.0-beta.4

- Rename `accept::openssl::{SslStream => TlsStream}`.
- Add `connect::Connect::set_local_addr` to attach local `IpAddr`. [#282]
- `connector::TcpConnector` service will try to bind to local_addr of `IpAddr` when given. [#282]

[#282]: https://github.com/actix/actix-net/pull/282

## 3.0.0-beta.3

- Remove `trust-dns-proto` and `trust-dns-resolver`. [#248]
- Use `std::net::ToSocketAddrs` as simple and basic default resolver. [#248]
- Add `Resolve` trait for custom DNS resolvers. [#248]
- Add `Resolver::new_custom` function to construct custom resolvers. [#248]
- Export `webpki_roots::TLS_SERVER_ROOTS` in `actix_tls::connect` mod and remove the export from `actix_tls::accept` [#248]
- Remove `ConnectTakeAddrsIter`. `Connect::take_addrs` now returns `ConnectAddrsIter<'static>` as owned iterator. [#248]
- Rename `Address::{host => hostname}` to more accurately describe which URL segment is returned.
- Update `actix-rt` to `2.0.0`. [#273]

[#248]: https://github.com/actix/actix-net/pull/248
[#273]: https://github.com/actix/actix-net/pull/273

## 3.0.0-beta.2

- Depend on stable trust-dns packages. [#204]

[#204]: https://github.com/actix/actix-net/pull/204

## 3.0.0-beta.1

- Move acceptors under `accept` module. [#238]
- Merge `actix-connect` crate under `connect` module. [#238]
- Add feature flags to enable acceptors and/or connectors individually. [#238]

[#238]: https://github.com/actix/actix-net/pull/238

## 2.0.0

- `nativetls::NativeTlsAcceptor` is renamed to `nativetls::Acceptor`.
- Where possible, "SSL" terminology is replaced with "TLS".
  - `SslError` is renamed to `TlsError`.
  - `TlsError::Ssl` enum variant is renamed to `TlsError::Tls`.
  - `max_concurrent_ssl_connect` is renamed to `max_concurrent_tls_connect`.

## 2.0.0-alpha.2

- Update `rustls` dependency to 0.18
- Update `tokio-rustls` dependency to 0.14
- Update `webpki-roots` dependency to 0.20

## [2.0.0-alpha.1]

- Update `rustls` dependency to 0.17
- Update `tokio-rustls` dependency to 0.13
- Update `webpki-roots` dependency to 0.19

## [1.0.0]

- 1.0.0 release

## [1.0.0-alpha.3]

- Migrate to tokio 0.2
- Enable rustls acceptor service
- Enable native-tls acceptor service

## [1.0.0-alpha.1]

- Split openssl acceptor from actix-server package
