# Changes

## Unreleased - 2021-xx-xx


## 2.0.2 - 2021-12-18
- Service types can now be `Send` and `'static` regardless of request, response, and config types, etc.. [#397]

[#397]: https://github.com/actix/actix-net/pull/397


## 2.0.1 - 2021-10-11
- Documentation fix.


## 2.0.0 - 2021-04-16
- Removed pipeline and related structs/functions. [#335]

[#335]: https://github.com/actix/actix-net/pull/335


## 2.0.0-beta.5 - 2021-03-15
- Add default `Service` trait impl for `Rc<S: Service>` and `&S: Service`. [#288]
- Add `boxed::rc_service` function for constructing `boxed::RcService` type [#290]

[#288]: https://github.com/actix/actix-net/pull/288
[#290]: https://github.com/actix/actix-net/pull/290


## 2.0.0-beta.4 - 2021-02-04
- `Service::poll_ready` and `Service::call` receive `&self`. [#247]
- `apply_fn` and `apply_fn_factory` now receive `Fn(Req, &Service)` function type. [#247]
- `apply_cfg` and `apply_cfg_factory` now receive `Fn(Req, &Service)` function type. [#247]
- `fn_service` and friends now receive `Fn(Req)` function type. [#247]

[#247]: https://github.com/actix/actix-net/pull/247


## 2.0.0-beta.3 - 2021-01-09
- The `forward_ready!` macro converts errors. [#246]

[#246]: https://github.com/actix/actix-net/pull/246


## 2.0.0-beta.2 - 2021-01-03
- Remove redundant type parameter from `map_config`.


## 2.0.0-beta.1 - 2020-12-28
- `Service`, other traits, and many type signatures now take the the request type as a type
  parameter instead of an associated type. [#232]
- Add `always_ready!` and `forward_ready!` macros. [#233]
- Crate is now `no_std`. [#233]
- Migrate pin projections to `pin-project-lite`. [#233]
- Remove `AndThenApplyFn` and Pipeline `and_then_apply_fn`. Use the
  `.and_then(apply_fn(...))` construction. [#233]
- Move non-vital methods to `ServiceExt` and `ServiceFactoryExt` extension traits. [#235]

[#232]: https://github.com/actix/actix-net/pull/232
[#233]: https://github.com/actix/actix-net/pull/233
[#235]: https://github.com/actix/actix-net/pull/235


## 1.0.6 - 2020-08-09
- Removed unsound custom Cell implementation that allowed obtaining several mutable references to
  the same data, which is undefined behavior in Rust and could lead to violations of memory safety. External code could obtain several mutable references to the same data through
  service combinators. Attempts to acquire several mutable references to the same data will instead
  result in a panic.


## 1.0.5 - 2020-01-16
- Fixed unsoundness in .and_then()/.then() service combinators.


## 1.0.4 - 2020-01-15
- Revert 1.0.3 change


## 1.0.3 - 2020-01-15
- Fixed unsoundness in `AndThenService` impl.


## 1.0.2 - 2020-01-08
- Add `into_service` helper function.


## 1.0.1 - 2019-12-22
- `map_config()` and `unit_config()` now accept `IntoServiceFactory` type.


## 1.0.0 - 2019-12-11
- Add Clone impl for Apply service


## 1.0.0-alpha.4 - 2019-12-08
- Renamed `service_fn` to `fn_service`
- Renamed `factory_fn` to `fn_factory`
- Renamed `factory_fn_cfg` to `fn_factory_with_config`


## 1.0.0-alpha.3 - 2019-12-06
- Add missing Clone impls
- Restore `Transform::map_init_err()` combinator
- Restore `Service/Factory::apply_fn()` in form of `Pipeline/Factory::and_then_apply_fn()`
- Optimize service combinators and futures memory layout


## 1.0.0-alpha.2 - 2019-12-02
- Use owned config value for service factory
- Renamed BoxedNewService/BoxedService to BoxServiceFactory/BoxService


## 1.0.0-alpha.1 - 2019-11-25
- Migrated to `std::future`
- `NewService` renamed to `ServiceFactory`
- Added `pipeline` and `pipeline_factory` function


## 0.4.2 - 2019-08-27
- Check service readiness for `new_apply_cfg` combinator


## 0.4.1 - 2019-06-06
- Add `new_apply_cfg` function


## 0.4.0 - 2019-05-12
- Add `NewService::map_config` and `NewService::unit_config` combinators.
- Use associated type for `NewService` config.
- Change `apply_cfg` function.
- Renamed helper functions.


## 0.3.6 - 2019-04-07
- Poll boxed service call result immediately


## 0.3.5 - 2019-03-29
- Add `impl<S: Service> Service for Rc<RefCell<S>>`.


## 0.3.4 - 2019-03-12
- Add `Transform::from_err()` combinator
- Add `apply_fn` helper
- Add `apply_fn_factory` helper
- Add `apply_transform` helper
- Add `apply_cfg` helper


## 0.3.3 - 2019-03-09
- Add `ApplyTransform` new service for transform and new service.
- Add `NewService::apply_cfg()` combinator, allows to use nested `NewService` with different config parameter.
- Revert IntoFuture change


## 0.3.2 - 2019-03-04
- Change `NewService::Future` and `Transform::Future` to the `IntoFuture` trait.
- Export `AndThenTransform` type


## 0.3.1 - 2019-03-04
- Simplify Transform trait


## 0.3.0 - 2019-03-02
- Added boxed NewService and Service.
- Added `Config` parameter to `NewService` trait.
- Added `Config` parameter to `NewTransform` trait.


## 0.2.2 - 2019-02-19
- Added `NewService` impl for `Rc<S> where S: NewService`
- Added `NewService` impl for `Arc<S> where S: NewService`


## 0.2.1 - 2019-02-03
- Generalize `.apply` combinator with Transform trait


## 0.2.0 - 2019-02-01
- Use associated type instead of generic for Service definition.
  * Before:
    ```rust
    impl Service<Request> for Client {
        type Response = Response;
        // ...
    }
    ```
  * After:
    ```rust
    impl Service for Client {
        type Request = Request;
        type Response = Response;
        // ...
    }
    ```


## 0.1.6 - 2019-01-24
- Use `FnMut` instead of `Fn` for .apply() and .map() combinators and `FnService` type
- Change `.apply()` error semantic, new service's error is `From<Self::Error>`


## 0.1.5 - 2019-01-13
- Make `Out::Error` convertible from `T::Error` for apply combinator


## 0.1.4 - 2019-01-11
- Use `FnMut` instead of `Fn` for `FnService`


## 0.1.3 - 2018-12-12
- Split service combinators to separate trait


## 0.1.2 - 2018-12-12
- Release future early for `.and_then()` and `.then()` combinators


## 0.1.1 - 2018-12-09
- Added Service impl for `Box<S: Service>`


## 0.1.0 - 2018-12-09
- Initial import
