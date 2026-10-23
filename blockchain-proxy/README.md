# nimiq-blockchain-proxy

`BlockchainProxy` and `BlockchainReadProxy`, which wrap either a full (`nimiq-blockchain`) or a light
(`nimiq-light-blockchain`) blockchain behind a single type, so that code such as consensus can work with both.

This crate is part of [Nimiq's Rust implementation](https://github.com/nimiq/core-rs-albatross) of the Albatross
protocol. Its version follows the releases of the Nimiq node.

## Documentation

The API documentation is available on [docs.rs](https://docs.rs/nimiq-blockchain-proxy).

## License

Licensed under Apache License, Version 2.0, (http://www.apache.org/licenses/LICENSE-2.0).
