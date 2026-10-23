# nimiq-time

Timers that work both natively and in WebAssembly: `sleep`, `sleep_until`, `interval` and `timeout`, backed by tokio
on native targets and by browser timers on `wasm32`.

This crate is part of [Nimiq's Rust implementation](https://github.com/nimiq/core-rs-albatross) of the Albatross
protocol. Its version follows the releases of the Nimiq node.

## Documentation

The API documentation is available on [docs.rs](https://docs.rs/nimiq-time).

## License

Licensed under Apache License, Version 2.0, (http://www.apache.org/licenses/LICENSE-2.0).
