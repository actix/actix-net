# Changes

## Unreleased - 2022-xx-xx


## 2.2.0 - 2022-12-21
- Minimum supported Rust version (MSRV) is now 1.59.
- Update `tokio-uring` dependency to `0.4`. [#473]

[#473]: https://github.com/actix/actix-net/pull/473


## 2.1.1 - 2022-03-09
- No significant changes since `2.1.0`.


## 2.1.0 - 2022-03-08
- Update `tokio-uring` dependency to `0.3`. [#448]
- Logs emitted now use the `tracing` crate with `log` compatibility. [#448]
- Wait for accept thread to stop before sending completion signal. [#443]

[#443]: https://github.com/actix/actix-net/pull/443
[#448]: https://github.com/actix/actix-net/pull/448


## 2.0.0 - 2022-01-19
- No significant changes since `2.0.0-rc.4`.


## 2.0.0-rc.4 - 2022-01-12
- Update `tokio-uring` dependency to `0.2`. [#436]

[#436]: https://github.com/actix/actix-net/pull/436


## 2.0.0-rc.3 - 2021-12-31
- No significant changes since `2.0.0-rc.2`.


## 2.0.0-rc.2 - 2021-12-27
- Simplify `TestServer`. [#431]

[#431]: https://github.com/actix/actix-net/pull/431


## 2.0.0-rc.1 - 2021-12-05
- Hide implementation details of `Server`. [#424]
- `Server` now runs only after awaiting it. [#425]

[#424]: https://github.com/actix/actix-net/pull/424
[#425]: https://github.com/actix/actix-net/pull/425


## 2.0.0-beta.9 - 2021-11-15
- Restore `Arbiter` support lost in `beta.8`. [#417]

[#417]: https://github.com/actix/actix-net/pull/417


## 2.0.0-beta.8 - 2021-11-05 _(YANKED)_
- Fix non-unix signal handler. [#410]

[#410]: https://github.com/actix/actix-net/pull/410


## 2.0.0-beta.7 - 2021-11-05 _(YANKED)_
- Server can be started in regular Tokio runtime. [#408]
- Expose new `Server` type whose `Future` impl resolves when server stops. [#408]
- Rename `Server` to `ServerHandle`. [#407]
- Add `Server::handle` to obtain handle to server. [#408]
- Rename `ServerBuilder::{maxconn => max_concurrent_connections}`. [#407]
- Deprecate crate-level `new` shortcut for server builder. [#408]
- Minimum supported Rust version (MSRV) is now 1.52.

[#407]: https://github.com/actix/actix-net/pull/407
[#408]: https://github.com/actix/actix-net/pull/408


## 2.0.0-beta.6 - 2021-10-11
- Add experimental (semver-exempt) `io-uring` feature for enabling async file I/O on linux. [#374]
- Server no long listens to `SIGHUP` signal. Previously, the received was not used but did block
  subsequent exit signals from working. [#389]
- Remove `config` module. `ServiceConfig`, `ServiceRuntime` public types are removed due to
  this change. [#349]
- Remove `ServerBuilder::configure` [#349]

[#374]: https://github.com/actix/actix-net/pull/374
[#349]: https://github.com/actix/actix-net/pull/349
[#389]: https://github.com/actix/actix-net/pull/389


## 2.0.0-beta.5 - 2021-04-20
- Server shutdown notifies all workers to exit regardless if shutdown is graceful. This causes all
  workers to shutdown immediately in force shutdown case. [#333]

[#333]: https://github.com/actix/actix-net/pull/333


## 2.0.0-beta.4 - 2021-04-01
- Prevent panic when `shutdown_timeout` is very large. [f9262db]

[f9262db]: https://github.com/actix/actix-net/commit/f9262db


## 2.0.0-beta.3 - 2021-02-06
- Hidden `ServerBuilder::start` method has been removed. Use `ServerBuilder::run`. [#246]
- Add retry for EINTR signal (`io::Interrupted`) in `Accept`'s poll loop. [#264]
- Add `ServerBuilder::worker_max_blocking_threads` to customize blocking thread pool size. [#265]
- Update `actix-rt` to `2.0.0`. [#273]

[#246]: https://github.com/actix/actix-net/pull/246
[#264]: https://github.com/actix/actix-net/pull/264
[#265]: https://github.com/actix/actix-net/pull/265
[#273]: https://github.com/actix/actix-net/pull/273


## 2.0.0-beta.2 - 2021-01-03
- Merge `actix-testing` to `actix-server` as `test_server` mod. [#242]

[#242]: https://github.com/actix/actix-net/pull/242


## 2.0.0-beta.1 - 2020-12-28
- Added explicit info log message on accept queue pause. [#215]
- Prevent double registration of sockets when back-pressure is resolved. [#223]
- Update `mio` dependency to `0.7.3`. [#239]
- Remove `socket2` dependency. [#239]
- `ServerBuilder::backlog` now accepts `u32` instead of `i32`. [#239]
- Remove `AcceptNotify` type and pass `WakerQueue` to `Worker` to wake up `Accept`'s `Poll`. [#239]
- Convert `mio::net::TcpStream` to `actix_rt::net::TcpStream`(`UnixStream` for uds) using
  `FromRawFd` and `IntoRawFd`(`FromRawSocket` and `IntoRawSocket` on windows). [#239]
- Remove `AsyncRead` and `AsyncWrite` trait bound for `socket::FromStream` trait. [#239]

[#215]: https://github.com/actix/actix-net/pull/215
[#223]: https://github.com/actix/actix-net/pull/223
[#239]: https://github.com/actix/actix-net/pull/239


## 1.0.4 - 2020-09-12
- Update actix-codec to 0.3.0.
- Workers must be greater than 0. [#167]

[#167]: https://github.com/actix/actix-net/pull/167


## 1.0.3 - 2020-05-19
- Replace deprecated `net2` crate with `socket2` [#140]

[#140]: https://github.com/actix/actix-net/pull/140


## 1.0.2 - 2020-02-26
- Avoid error by calling `reregister()` on Windows [#103]

[#103]: https://github.com/actix/actix-net/pull/103


## 1.0.1 - 2019-12-29
- Rename `.start()` method to `.run()`


## 1.0.0 - 2019-12-11
- Use actix-net releases


## 1.0.0-alpha.4 - 2019-12-08
- Use actix-service 1.0.0-alpha.4


## 1.0.0-alpha.3 - 2019-12-07
- Migrate to tokio 0.2
- Fix compilation on non-unix platforms
- Better handling server configuration


## 1.0.0-alpha.2 - 2019-12-02
- Simplify server service (remove actix-server-config)
- Allow to wait on `Server` until server stops


## 0.8.0-alpha.1 - 2019-11-22
- Migrate to `std::future`


## 0.7.0 - 2019-10-04
- Update `rustls` to 0.16
- Minimum required Rust version upped to 1.37.0


## 0.6.1 - 2019-09-25
- Add UDS listening support to `ServerBuilder`


## 0.6.0 - 2019-07-18
- Support Unix domain sockets #3


## 0.5.1 - 2019-05-18
- ServerBuilder::shutdown_timeout() accepts u64


## 0.5.0 - 2019-05-12
- Add `Debug` impl for `SslError`
- Derive debug for `Server` and `ServerCommand`
- Upgrade to actix-service 0.4


## 0.4.3 - 2019-04-16
- Re-export `IoStream` trait
- Depend on `ssl` and `rust-tls` features from actix-server-config


## 0.4.2 - 2019-03-30
- Fix SIGINT force shutdown


## 0.4.1 - 2019-03-14
- `SystemRuntime::on_start()` - allow to run future before server service initialization


## 0.4.0 - 2019-03-12
- Use `ServerConfig` for service factory
- Wrap tcp socket to `Io` type
- Upgrade actix-service


## 0.3.1 - 2019-03-04
- Add `ServerBuilder::maxconnrate` sets the maximum per-worker number of concurrent connections
- Add helper ssl error `SslError`
- Rename `StreamServiceFactory` to `ServiceFactory`
- Deprecate `StreamServiceFactory`


## 0.3.0 - 2019-03-02
- Use new `NewService` trait


## 0.2.1 - 2019-02-09
- Drop service response


## 0.2.0 - 2019-02-01
- Migrate to actix-service 0.2
- Updated rustls dependency


## 0.1.3 - 2018-12-21
- Fix max concurrent connections handling


## 0.1.2 - 2018-12-12
- rename ServiceConfig::rt() to ServiceConfig::apply()
- Fix back-pressure for concurrent ssl handshakes


## 0.1.1 - 2018-12-11
- Fix signal handling on windows


## 0.1.0 - 2018-12-09
- Move server to separate crate
