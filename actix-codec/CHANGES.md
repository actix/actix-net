# Changes

## Unreleased

## 0.5.2

- Minimum supported Rust version (MSRV) is now 1.65.

## 0.5.1

- Logs emitted now use the `tracing` crate with `log` compatibility.
- Minimum supported Rust version (MSRV) is now 1.49.

## 0.5.0

- Updated `tokio-util` dependency to `0.7.0`.

## 0.4.2

- No significant changes since `0.4.1`.

## 0.4.1

- Added `LinesCodec`.
- `Framed::poll_ready` flushes when the buffer is full.

## 0.4.0

- No significant changes since v0.4.0-beta.1.

## 0.4.0-beta.1

- Replace `pin-project` with `pin-project-lite`.
- Upgrade `tokio` dependency to `1`.
- Upgrade `tokio-util` dependency to `0.6`.
- Upgrade `bytes` dependency to `1`.

## 0.3.0

- No changes from beta 2.

## 0.3.0-beta.2

- Remove unused type parameter from `Framed::replace_codec`.

## 0.3.0-beta.1

- Use `.advance()` instead of `.split_to()`.
- Upgrade `tokio-util` to `0.3`.
- Improve `BytesCodec::encode()` performance.
- Simplify `BytesCodec::decode()`.
- Rename methods on `Framed` to better describe their use.
- Add method on `Framed` to get a pinned reference to the underlying I/O.
- Add method on `Framed` check emptiness of read buffer.

## 0.2.0

- Use specific futures dependencies.

## 0.2.0-alpha.4

- Fix buffer remaining capacity calculation.

## 0.2.0-alpha.3

- Use tokio 0.2.
- Fix low/high watermark for write/read buffers.

## 0.2.0-alpha.2

- Migrated to `std::future`.

## 0.1.2

- Added `Framed::map_io()` method.

## 0.1.1

- Added `FramedParts::with_read_buffer()` method.

## 0.1.0

- Move codec to separate crate.
