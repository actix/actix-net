# Changes

## Unreleased - 2022-xx-xx


## 0.5.1 - 2022-03-15
- Logs emitted now use the `tracing` crate with `log` compatibility. [#451]
- Minimum supported Rust version (MSRV) is now 1.49.

[#451]: https://github.com/actix/actix-net/pull/451


## 0.5.0 - 2022-02-15
- Updated `tokio-util` dependency to `0.7.0`. [#446]

[#446]: https://github.com/actix/actix-net/pull/446


## 0.4.2 - 2021-12-31
- No significant changes since `0.4.1`.


## 0.4.1 - 2021-11-05
- Added `LinesCodec.` [#338]
- `Framed::poll_ready` flushes when the buffer is full. [#409]

[#338]: https://github.com/actix/actix-net/pull/338
[#409]: https://github.com/actix/actix-net/pull/409


## 0.4.0 - 2021-04-20
- No significant changes since v0.4.0-beta.1.


## 0.4.0-beta.1 - 2020-12-28
- Replace `pin-project` with `pin-project-lite`. [#237]
- Upgrade `tokio` dependency to `1`. [#237]
- Upgrade `tokio-util` dependency to `0.6`. [#237]
- Upgrade `bytes` dependency to `1`. [#237]

[#237]: https://github.com/actix/actix-net/pull/237


## 0.3.0 - 2020-08-23
- No changes from beta 2.


## 0.3.0-beta.2 - 2020-08-19
- Remove unused type parameter from `Framed::replace_codec`.


## 0.3.0-beta.1 - 2020-08-19
- Use `.advance()` instead of `.split_to()`.
- Upgrade `tokio-util` to `0.3`.
- Improve `BytesCodec::encode()` performance.
- Simplify `BytesCodec::decode()`.
- Rename methods on `Framed` to better describe their use.
- Add method on `Framed` to get a pinned reference to the underlying I/O.
- Add method on `Framed` check emptiness of read buffer.


## 0.2.0 - 2019-12-10
- Use specific futures dependencies.


## 0.2.0-alpha.4
- Fix buffer remaining capacity calculation.


## 0.2.0-alpha.3
- Use tokio 0.2.
- Fix low/high watermark for write/read buffers.


## 0.2.0-alpha.2
- Migrated to `std::future`.


## 0.1.2 - 2019-03-27
- Added `Framed::map_io()` method.


## 0.1.1 - 2019-03-06
- Added `FramedParts::with_read_buffer()` method.


## 0.1.0 - 2018-12-09
- Move codec to separate crate.
