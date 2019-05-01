# Nimiq Core implementation in Rust _(core-rs)_

![nimiq](docs/nimiq_logo_rgb_horizontal.png)

> Rust implementation of the Nimiq Blockchain Core

**[Nimiq](https://nimiq.com/)**  is a frictionless payment protocol for the web.

This repository is **Work in Progress** and is currently in beta-testing phase. If you need a reliable client implementation to run in a production environment, please use the [JavaScript implementation](https://github.com/nimiq-network/core/) instead. Only use this if you can tolerate bugs and want to help beta-testing the Nimiq Rust implementation.

The Nimiq Rust client comes without wallet and can currently not be used to send transactions. As a back-bone node it is more performant that the JavaScript implementation though. 

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

### From crates.io

To download from [crates.io](https://crates.io), compile and install the client:

```bash
cargo +nightly install nimiq-client
```

The binary will be installed in your Cargo directory, which is usually at `$HOME/.cargo/bin`, and should be available in your `$PATH`.

### From Git

Compiling the project is achieved through [`cargo`](https://doc.rust-lang.org/cargo/):

```bash
git clone https://github.com/nimiq/core-rs
cd core-rs
cargo +nightly build
```

Note that this will build it in debug mode, which is not as performant. To get the most speed out of the client, build it in release mode:

```bash
cargo +nightly build --release
```

If you want to install the client onto your system (into `$HOME/.cargo/bin`), run:

```bash
cargo +nightly install --path client/
```

Alternatively you can install directly from git:

```bash
cargo +nightly install --git https://github.com/nimiq/core-rs.git
```


After installing the client you can use it like you had downloaded it from [crates.io](https://crates.io).

## Usage


### Configuration

By default the client will look for a configuration file in `$HOME/.nimiq/client.config`. You need to create this file yourself:

```bash
nimiq-client                                                   # Run the client. This will create the example config file.
cp $HOME/.nimiq/client.example.toml $HOME/.nimiq/client.toml   # Create the config from the example.
nano $HOME/.nimiq/client.toml                                  # Edit the config. Explanations are included in the file.
```

You can also specify your own configuration file:

```bash
nimiq-client -c path/to/client.toml
```

Take a look at [`client/client.example.toml`](client/config.example.toml) for all the configuration options.

### From crates.io

If you installed the client from [crates.io](https://crates.io), you can just run it with:

```bash
nimiq-client
```

### From Git

To run the Nimiq Rust Client from the downloaded sources:

```bash
cd core-rs/client
cargo run
```

Dependencies and binaries will be downloaded and compiled automatically by Cargo.

If you want to use the release build:

```bash
cd core-rs/client
cargo run --release
```

## Contributing

If you'd like to contribute to the development of Nimiq please follow our [Code of Conduct](/.github/CODE_OF_CONDUCT.md) and [Contributing Guidelines](/.github/CONTRIBUTING.md).

Small note: If editing the README, please conform to the [standard-readme](https://github.com/RichardLitt/standard-readme) specification.

## License

This project is under the [Apache License 2.0](./LICENSE.md).
