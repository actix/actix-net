# Changes

## Unreleased - 2021-xx-xx
* Resource definitions with unnamed tail segments now correctly interpolate the tail when constructed from an iterator. [#371]
* Introduce `ResourceDef::resource_path_from_map_with_tail` method to allow building paths in the presence of unnamed tail segments. [#371]
* Fix segment interpolation leaving `Path` in unintended state after matching. [#368]
* Path tail pattern now works as expected after a dynamic segment (e.g. `/user/{uid}/*`). [#366]
* Fixed a bug where, in multi-patterns, static patterns are interpreted as regex. [#366]
* Re-work `IntoPatterns` trait. [#372]
* Rename `Path::{len => segment_count}` to be more descriptive of it's purpose. [#370]
* Alias `ResourceDef::{resource_path => resource_path_from_iter}` pending eventual deprecation. [#371]
* Alias `ResourceDef::{resource_path_named => resource_path_from_map}` pending eventual deprecation. [#371]

[#368]: https://github.com/actix/actix-net/pull/368
[#366]: https://github.com/actix/actix-net/pull/366
[#368]: https://github.com/actix/actix-net/pull/368
[#370]: https://github.com/actix/actix-net/pull/370
[#371]: https://github.com/actix/actix-net/pull/371
[#372]: https://github.com/actix/actix-net/pull/372


## 0.4.0 - 2021-06-06
* When matching path parameters, `%25` is now kept in the percent-encoded form; no longer decoded to `%`. [#357]
* Path tail patterns now match new lines (`\n`) in request URL. [#360]
* Fixed a safety bug where `Path` could return a malformed string after percent decoding. [#359]
* Methods `Path::{add, add_static}` now take `impl Into<Cow<'static, str>>`. [#345]

[#345]: https://github.com/actix/actix-net/pull/345
[#357]: https://github.com/actix/actix-net/pull/357
[#359]: https://github.com/actix/actix-net/pull/359
[#360]: https://github.com/actix/actix-net/pull/360


## 0.3.0 - 2019-12-31
* Version was yanked previously. See https://crates.io/crates/actix-router/0.3.0


## 0.2.7 - 2021-02-06
* Add `Router::recognize_checked` [#247]

[#247]: https://github.com/actix/actix-net/pull/247


## 0.2.6 - 2021-01-09
* Use `bytestring` version range compatible with Bytes v1.0. [#246]

[#246]: https://github.com/actix/actix-net/pull/246


## 0.2.5 - 2020-09-20
* Fix `from_hex()` method


## 0.2.4 - 2019-12-31
* Add `ResourceDef::resource_path_named()` path generation method


## 0.2.3 - 2019-12-25
* Add impl `IntoPattern` for `&String`


## 0.2.2 - 2019-12-25
* Use `IntoPattern` for `RouterBuilder::path()`


## 0.2.1 - 2019-12-25
* Add `IntoPattern` trait
* Add multi-pattern resources


## 0.2.0 - 2019-12-07
* Update http to 0.2
* Update regex to 1.3
* Use bytestring instead of string


## 0.1.5 - 2019-05-15
* Remove debug prints


## 0.1.4 - 2019-05-15
* Fix checked resource match


## 0.1.3 - 2019-04-22
* Added support for `remainder match` (i.e "/path/{tail}*")


## 0.1.2 - 2019-04-07
* Export `Quoter` type
* Allow to reset `Path` instance


## 0.1.1 - 2019-04-03
* Get dynamic segment by name instead of iterator.


## 0.1.0 - 2019-03-09
* Initial release
