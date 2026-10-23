# nimiq-consensus

The consensus layer of a Nimiq node. It keeps track of the consensus state and synchronizes the blockchain with peers:
history, light and pico sync to catch up, then live sync to follow the chain. It also defines the network messages used
for synchronization.

This crate is part of [Nimiq's Rust implementation](https://github.com/nimiq/core-rs-albatross) of the Albatross
protocol. Its version follows the releases of the Nimiq node.

## Documentation

The API documentation is available on [docs.rs](https://docs.rs/nimiq-consensus).

## License

Licensed under Apache License, Version 2.0, (http://www.apache.org/licenses/LICENSE-2.0).
