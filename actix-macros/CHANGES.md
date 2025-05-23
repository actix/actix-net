# Changes

## Unreleased

- Minimum supported Rust version (MSRV) is now 1.74.

## 0.2.4

- Update `syn` dependency to `2`.
- Minimum supported Rust version (MSRV) is now 1.65.

## 0.2.3

- Fix test macro in presence of other imports named "test". [#399]

[#399]: https://github.com/actix/actix-net/pull/399

## 0.2.2

- Improve error recovery potential when macro input is invalid. [#391]
- Allow custom `System`s on test macro. [#391]

[#391]: https://github.com/actix/actix-net/pull/391

## 0.2.1

- Add optional argument `system` to `main` macro which can be used to specify the path to `actix_rt::System` (useful for re-exports). [#363]

[#363]: https://github.com/actix/actix-net/pull/363

## 0.2.0

- Update to latest `actix_rt::System::new` signature. [#261]

[#261]: https://github.com/actix/actix-net/pull/261

## 0.2.0-beta.1

- Remove `actix-reexport` feature. [#218]

[#218]: https://github.com/actix/actix-net/pull/218

## 0.1.3

- Add `actix-reexport` feature. [#218]

[#218]: https://github.com/actix/actix-net/pull/218

## 0.1.2

- Forward actix_rt::test arguments to test function [#127]

[#127]: https://github.com/actix/actix-net/pull/127
