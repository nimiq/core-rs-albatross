# Nimiq Core implementation in Rust _(core-rs)_

![nimiq](docs/nimiq_logo_rgb_horizontal.png)

> Rust implementation of the Nimiq Blockchain Core

**[Nimiq](https://nimiq.com/)**  is a frictionless payment protocol for the web.

This repository is **Work in Progress** and should only be used for testing purposes, it is not production ready yet. If you need a client implementation to run in a production environment, please use the [JavaScript implementation](https://github.com/nimiq-network/core/) instead. 

## Table of Contents

- [Background](#background)
- [Install](#install)
- [Usage](#usage)
- [Contributing](#contributing)
- [License](#license)

## Background

- [Nimiq White Paper](https://medium.com/nimiq-network/nimiq-a-peer-to-peer-payment-protocol-native-to-the-web-ffd324bb084): High-level introduction of the Nimiq payment protocol.
- [Nimiq Developer Reference](https://nimiq-network.github.io/developer-reference/): Details of the protocol architecture.
- [Testnet](https://nimiq-testnet.com): Demo of the Nimiq ecosystem in a test version of the network.


## Install

Besides [Rust](https://www.rust-lang.org/learn/get-started#installing-rust) itself, the following packages are required to be able to compile this source code:

- `gcc`
- `pkg-config`
- `libssl-dev` (in Debian/Ubuntu) or `openssl-dev` (in Fedora/Red Hat)

Compiling the project is achieved through [`cargo`](https://doc.rust-lang.org/cargo/):

```
git clone https://github.com/nimiq/core-rs
cd core-rs
cargo build
```

## Usage

Once compiled, you can run the Nimiq Rust Client directly or through `cargo run`:

```
cargo run --bin nimiq
```

To display log output, set the `RUST_LOG` environment variable: `RUST_LOG=nimiq=debug`.

## Contributing

If you'd like to contribute to the development of Nimiq please follow our [Code of Conduct](/.github/CODE_OF_CONDUCT.md) and [Contributing Guidelines](/.github/CONTRIBUTING.md).

Small note: If editing the README, please conform to the [standard-readme](https://github.com/RichardLitt/standard-readme) specification.

## License

This project is under the [Apache License 2.0](./LICENSE.md).
