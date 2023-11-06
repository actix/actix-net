# Changes

## Unreleased

## 1.3.1

- No significant changes since `1.3.0`.

## 1.3.0

- Implement `AsRef<ByteString>` for `ByteString`.

## 1.2.1

- Fix `#[no_std]` compatibility. [#471]

[#471]: https://github.com/actix/actix-net/pull/471

## 1.2.0

- Add `ByteString::slice_ref` which can safely slice a `ByteString` into a new one with zero copy. [#470]
- Minimum supported Rust version (MSRV) is now 1.57.

[#470]: https://github.com/actix/actix-net/pull/470

## 1.1.0

- Implement `From<Box<str>>` for `ByteString`. [#458]
- Implement `Into<String>` for `ByteString`. [#458]
- Minimum supported Rust version (MSRV) is now 1.49.

[#458]: https://github.com/actix/actix-net/pull/458

## 1.0.0

- Update `bytes` dependency to `1`.
- Add array and slice of `u8` impls of `TryFrom` up to 32 in length.
- Rename `get_ref` to `as_bytes` and rename `into_inner` to `into_bytes`.
- `ByteString::new` is now a `const fn`.
- Crate is now `#[no_std]` compatible.

## 0.1.5

- Serde support

## 0.1.4

- Fix `AsRef<str>` impl

## 0.1.3

- Add `PartialEq<T: AsRef<str>>`, `AsRef<[u8]>` impls

## 0.1.2

- Fix `new()` method
- Make `ByteString::from_static()` and `ByteString::from_bytes_unchecked()` methods const.

## 0.1.1

- Fix hash impl

## 0.1.0

- Initial release
