# nimiq-dht

The verifier for records published in Nimiq's DHT. It checks validator records against the current validator set of
the blockchain and rejects records with invalid signatures or timestamps too far in the future.

This crate is part of [Nimiq's Rust implementation](https://github.com/nimiq/core-rs-albatross) of the Albatross
protocol. Its version follows the releases of the Nimiq node.

## Documentation

The API documentation is available on [docs.rs](https://docs.rs/nimiq-dht).

## License

Licensed under Apache License, Version 2.0, (http://www.apache.org/licenses/LICENSE-2.0).
