# nimiq-mempool

The mempool of a Nimiq node. It verifies incoming transactions, keeps them ordered by fee, applies the configured
filters, and hands validators the transactions to include in a block.

This crate is part of [Nimiq's Rust implementation](https://github.com/nimiq/core-rs-albatross) of the Albatross
protocol. Its version follows the releases of the Nimiq node.

## Documentation

The API documentation is available on [docs.rs](https://docs.rs/nimiq-mempool).

## License

Licensed under Apache License, Version 2.0, (http://www.apache.org/licenses/LICENSE-2.0).
