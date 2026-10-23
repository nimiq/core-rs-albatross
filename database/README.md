# nimiq-database

A wrapper around [MDBX](https://libmdbx.dqdkfa.ru/) used for persistent storage in Nimiq. It provides typed tables,
read and write transactions, cursors, and volatile (temporary) databases for tests and in-memory use.

This crate is part of [Nimiq's Rust implementation](https://github.com/nimiq/core-rs-albatross) of the Albatross
protocol. Its version follows the releases of the Nimiq node.

## Documentation

The API documentation is available on [docs.rs](https://docs.rs/nimiq-database).

## License

Licensed under Apache License, Version 2.0, (http://www.apache.org/licenses/LICENSE-2.0).
