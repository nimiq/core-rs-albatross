# nimiq-genesis-builder

Builds a Nimiq genesis block and its initial accounts, validators and stakers from a TOML configuration. It is used by
`nimiq-genesis` at build time and also ships the `nimiq-genesis-builder` binary for creating new networks.

This crate is part of [Nimiq's Rust implementation](https://github.com/nimiq/core-rs-albatross) of the Albatross
protocol. Its version follows the releases of the Nimiq node.

## Documentation

The API documentation is available on [docs.rs](https://docs.rs/nimiq-genesis-builder).

## License

Licensed under Apache License, Version 2.0, (http://www.apache.org/licenses/LICENSE-2.0).
