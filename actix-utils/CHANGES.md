# Changes

## Unreleased - 2022-xx-xx


## 3.0.1 - 2022-10-21
- Minimum supported Rust version (MSRV) is now 1.57.


## 3.0.0 - 2021-04-16
- No significant changes from `3.0.0-beta.4`.


## 3.0.0-beta.4 - 2021-04-01
- Add `future::Either` type. [#305]

[#305]: https://github.com/actix/actix-net/pull/305


## 3.0.0-beta.3 - 2021-04-01
- Moved `mpsc` to own crate `local-channel`. [#301]
- Moved `task::LocalWaker` to own crate `local-waker`. [#301]
- Remove `timeout` module. [#301]
- Remove `dispatcher` module. [#301]
- Expose `future` mod with `ready` and `poll_fn` helpers. [#301]

[#301]: https://github.com/actix/actix-net/pull/301


## 3.0.0-beta.2 - 2021-02-06
- Update `actix-rt` to `2.0.0`. [#273]

[#273]: https://github.com/actix/actix-net/pull/273


## 3.0.0-beta.1 - 2020-12-28
- Update `bytes` dependency to `1`. [#237]
- Use `pin-project-lite` to replace `pin-project`. [#229]
- Remove `condition`,`either`,`inflight`,`keepalive`,`oneshot`,`order`,`stream` and `time` mods. [#229]

[#229]: https://github.com/actix/actix-net/pull/229
[#237]: https://github.com/actix/actix-net/pull/237


## 2.0.0 - 2020-08-23
- No changes from beta 1.


## 2.0.0-beta.1 - 2020-08-19
- Upgrade `tokio-util` to `0.3`.
- Remove unsound custom Cell and use `std::cell::RefCell` instead, as well as `actix-service`.
- Rename method to correctly spelled `LocalWaker::is_registered`.


## 1.0.6 - 2020-01-08
- Add `Clone` impl for `condition::Waiter`.


## 1.0.5 - 2020-01-08
- Add `Condition` type.
- Add `Pool` of one-shot's.


## 1.0.4 - 2019-12-20
- Add methods to check `LocalWaker` registration state.


## 1.0.3 - 2019-12-11
- Revert InOrder service changes


## 1.0.2 - 2019-12-11
- Allow to create `framed::Dispatcher` with custom `mpsc::Receiver`.
- Add `oneshot::Sender::is_canceled()` method.


## 1.0.1 - 2019-12-11
- Optimize InOrder service.


## 1.0.0 - 2019-12-11
- Simplify oneshot and mpsc implementations.


## 1.0.0-alpha.3 - 2019-12-07
- Migrate to tokio 0.2.
- Fix oneshot.


## 1.0.0-alpha.2 - 2019-12-02
- Migrate to `std::future`.


## 0.4.7 - 2019-10-14
- Re-register task on every framed transport poll.


## 0.4.6 - 2019-10-08
- Refactor `Counter` type. register current task in available method.


## 0.4.5 - 2019-07-19
- Deprecated `CloneableService` as it is not safe.


## 0.4.4 - 2019-07-17
- Undeprecate `FramedTransport` as it is actually useful.


## 0.4.3 - 2019-07-17
- Deprecate `CloneableService` as it is not safe and in general not very useful.
- Deprecate `FramedTransport` in favor of `actix-ioframe`.


## 0.4.2 - 2019-06-26
- Do not block on sink drop for FramedTransport.


## 0.4.1 - 2019-05-15
- Change `Either` constructor.


## 0.4.0 - 2019-05-11
- Change `Either` to handle two nexted services.
- Upgrade actix-service 0.4.
- Removed framed related services.
- Removed stream related services.


## 0.3.5 - 2019-04-04
- Allow to send messages to `FramedTransport` via mpsc channel.
- Remove `'static` constraint from Clonable service.


## 0.3.4 - 2019-03-12
- `TimeoutService`, `InOrderService`, `InFlightService` accepts generic IntoService services.
- Fix `InFlightService::poll_ready()` nested service readiness check.
- Fix `InOrderService::poll_ready()` nested service readiness check.


## 0.3.3 - 2019-03-09
- Revert IntoFuture change.
- Add generic config param for IntoFramed and TakeOne new services.


## 0.3.2 - 2019-03-04
- Use IntoFuture for new services.

## 0.3.1 - 2019-03-04
- Use new type of transform trait.


## 0.3.0 - 2019-03-02
- Use new `NewService` trait
- BoxedNewService` and `BoxedService` types moved to actix-service crate.


## 0.2.4 - 2019-02-21
- Custom `BoxedNewService` implementation.


## 0.2.3 - 2019-02-21
- Add `BoxedNewService` and `BoxedService`.


## 0.2.2 - 2019-02-11
- Add `Display` impl for `TimeoutError`.
- Add `Display` impl for `InOrderError`.


## 0.2.1 - 2019-02-06
- Add `InOrder` service. the service yields responses as they become available,
  in the order that their originating requests were submitted to the service.
- Convert `Timeout` and `InFlight` services to a transforms.


## 0.2.0 - 2019-02-01
- Fix framed transport error handling.
- Added Clone impl for Either service.
- Added Clone impl for Timeout service factory.
- Added Service and NewService for Stream dispatcher.
- Switch to actix-service 0.2.


## 0.1.0 - 2018-12-09
- Move utils services to separate crate.
