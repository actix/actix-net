# Changes

## Unreleased

## 2.10.0

- Relax bound (`F: Fn -> FnOnce`) on `{Arbiter, System}::with_tokio_rt()` functions.
- Update `tokio-uring` dependency to `0.5`.
- Minimum supported Rust version (MSRV) is now 1.70.

## 2.9.0

- Add `actix_rt::System::runtime()` method to retrieve the underlying `actix_rt::Runtime` runtime.
- Add `actix_rt::Runtime::tokio_runtime()` method to retrieve the underlying Tokio runtime.
- Minimum supported Rust version (MSRV) is now 1.65.

## 2.8.0

- Add `#[track_caller]` attribute to `spawn` functions and methods.
- Update `tokio-uring` dependency to `0.4`.
- Minimum supported Rust version (MSRV) is now 1.59.

## 2.7.0

- Update `tokio-uring` dependency to `0.3`.
- Minimum supported Rust version (MSRV) is now 1.49.

## 2.6.0

- Update `tokio-uring` dependency to `0.2`.

## 2.5.1

- Expose `System::with_tokio_rt` and `Arbiter::with_tokio_rt`.

## 2.5.0

- Add `System::run_with_code` to allow retrieving the exit code on stop.

## 2.4.0

- Add `Arbiter::try_current` for situations where thread may or may not have Arbiter context.
- Start io-uring with `System::new` when feature is enabled.

## 2.3.0

- The `spawn` method can now resolve with non-unit outputs.
- Add experimental (semver-exempt) `io-uring` feature for enabling async file I/O on linux.

## 2.2.0

- **BREAKING** `ActixStream::{poll_read_ready, poll_write_ready}` methods now return `Ready` object in ok variant.
  - Breakage is acceptable since `ActixStream` was not intended to be public.

## 2.1.0

- Add `ActixStream` extension trait to include readiness methods.
- Re-export `tokio::net::TcpSocket` in `net` module

## 2.0.2

- Add `Arbiter::handle` to get a handle of an owned Arbiter.
- Add `System::try_current` for situations where actix may or may not be running a System.

## 2.0.1

- Expose `JoinError` from Tokio.

## 2.0.0

- Remove all Arbiter-local storage methods.
- Re-export `tokio::pin`.

## 2.0.0-beta.3

- Remove `run_in_tokio`, `attach_to_tokio` and `AsyncSystemRunner`.
- Return `JoinHandle` from `actix_rt::spawn`.
- Remove old `Arbiter::spawn`. Implementation is now inlined into `actix_rt::spawn`.
- Rename `Arbiter::{send => spawn}` and `Arbiter::{exec_fn => spawn_fn}`.
- Remove `Arbiter::exec`.
- Remove deprecated `Arbiter::local_join` and `Arbiter::is_running`.
- `Arbiter::spawn` now accepts !Unpin futures.
- `System::new` no longer takes arguments.
- Remove `System::with_current`.
- Remove `Builder`.
- Add `System::with_init` as replacement for `Builder::run`.
- Rename `System::{is_set => is_registered}`.
- Add `ArbiterHandle` for sending messages to non-current-thread arbiters.
- `System::arbiter` now returns an `&ArbiterHandle`.
- `Arbiter::current` now returns an `ArbiterHandle` instead.
- `Arbiter::join` now takes self by value.

## 2.0.0-beta.2

- Add `task` mod with re-export of `tokio::task::{spawn_blocking, yield_now, JoinHandle}`
- Add default "macros" feature to allow faster compile times when using `default-features=false`.

## 2.0.0-beta.1

- Add `System::attach_to_tokio` method.
- Update `tokio` dependency to `1.0`.
- Rename `time` module `delay_for` to `sleep`, `delay_until` to `sleep_until`, `Delay` to `Sleep` to stay aligned with Tokio's naming.
- Remove `'static` lifetime requirement for `Runtime::block_on` and `SystemRunner::block_on`.
  - These methods now accept `&self` when calling.
- Remove `'static` lifetime requirement for `System::run` and `Builder::run`.
- `Arbiter::spawn` now panics when `System` is not in scope.
- Fix work load issue by removing `PENDING` thread local.

## 1.1.1

- Fix memory leak due to

## 1.1.0 _(YANKED)_

- Expose `System::is_set` to check if current system has ben started
- Add `Arbiter::is_running` to check if event loop is running
- Add `Arbiter::local_join` associated function to get be able to `await` for spawned futures

## 1.0.0

- Update dependencies

## 1.0.0-alpha.3

- Migrate to tokio 0.2
- Fix compilation on non-unix platforms

## 1.0.0-alpha.2

- Export `main` and `test` attribute macros
- Export `time` module (re-export of tokio-timer)
- Export `net` module (re-export of tokio-net)

## 1.0.0-alpha.1

- Migrate to std::future and tokio 0.2

## 0.2.6

- Allow to join arbiter's thread. #60
- Fix arbiter's thread panic message.

## 0.2.5

- Add arbiter specific storage

## 0.2.4

- Avoid a copy of the Future when initializing the Box. #29

## 0.2.3

- Allow to start System using existing CurrentThread Handle #22

## 0.2.2

- Moved `blocking` module to `actix-threadpool` crate

## 0.2.1

- Added `blocking` module
- Added `Arbiter::exec_fn` - execute fn on the arbiter's thread
- Added `Arbiter::exec` - execute fn on the arbiter's thread and wait result

## 0.2.0

- `run` method returns `io::Result<()>`
- Removed `Handle`

## 0.1.0

- Initial release
