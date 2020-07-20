# Changes

## Unreleased - 2020-xx-xx
* Use `.advance()` instead of `.split_to()`.
* Upgrade `tokio-util` to `0.3`.
* Improve `BytesCodec` `.encode()` performance
* Simplify `BytesCodec` `.decode()` 

## [0.2.0] - 2019-12-10

* Use specific futures dependencies

## [0.2.0-alpha.4]

* Fix buffer remaining capacity calculation

## [0.2.0-alpha.3]

* Use tokio 0.2

* Fix low/high watermark for write/read buffers

## [0.2.0-alpha.2]

* Migrated to `std::future`

## [0.1.2] - 2019-03-27

* Added `Framed::map_io()` method.

## [0.1.1] - 2019-03-06

* Added `FramedParts::with_read_buffer()` method.

## [0.1.0] - 2018-12-09

* Move codec to separate crate
