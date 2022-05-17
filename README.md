# Nimiq 2.0 implementation in Rust

![nimiq](https://raw.githubusercontent.com/nimiq/designs/master/logo/RGB/colored/png/nimiq_logo_rgb_horizontal.png)

> Rust implementation of the Nimiq 2.0 blockchain node

[![Build Status](https://github.com/nimiq/core-rs-albatross/actions/workflows/build+test.yml/badge.svg?branch=albatross)](https://github.com/nimiq/core-rs-albatross/actions/workflows/build+test.yml?query=branch%3Aalbatross)
[![dependency status](https://deps.rs/repo/github/nimiq/core-rs-albatross/status.svg)](https://deps.rs/repo/github/nimiq/core-rs-albatross)

**[Nimiq](https://nimiq.com/)**  is a frictionless payment protocol for the web.

This repository is currently under development. It contains the implementation of the Nimiq 2.0 protocol:
a Proof-of-stake blockchain based on the [Albatross](https://arxiv.org/abs/1903.01589) consensus algorithm.

Nimiq 2.0 was conceived with performance in mind without sacrificing security.

Currently, the protocol can be exercised in an environment aimed for developers where bugs are expected to happen.
For more detailed information about how to connect and use the development network, please refer to the [Devnet](#devnet) section.

## Table of Contents

- [Background](#background)
- [System requirements](#system-requirements)
- [Installation](#installation)
- [Usage](#usage)
- [Configuration](#configuration)
- [Devnet](#devnet)
- [Contributing](#contributing)
- [License](#license)

## Background

- [Nimiq 2.0 Portal](https://www.nimiq.com/albatross/): General information and high level details of the Nimiq 2.0 blockchain
- [Nimiq Albatross White Paper](https://arxiv.org/abs/1903.01589): White paper describing the consensus algorithm used in Nimiq 2.0
- [Nimiq 2.0 migration process](https://www.nimiq.com/blog/nimiq-20-albatross-hard-fork-preparations/): Migration process from Nimiq 1.0 to 2.0
- [Nimiq 1.0 Developer Reference](https://nimiq-network.github.io/developer-reference/): Details of the protocol architecture.
- [Nimiq 1.0 JavaScript implementation](https://github.com/nimiq/core-js/): Nimiq 1.0 implementation


## System requirements
- 64-bit computing architecture.
- File systems with sparse file support.


## Installation

Besides [Rust stable](https://www.rust-lang.org/learn/get-started#installing-rust) itself,
the following packages are required to be able to compile the source code:

- `clang`
- `pkg-config`
- `libssl-dev` (in Debian/Ubuntu) or `openssl-devel` (in Fedora/Red Hat)


After installing the previous packages, compiling the project is achieved through [`cargo`](https://doc.rust-lang.org/cargo/):

```bash
git clone https://github.com/nimiq/core-rs-albatross
cd core-rs
cargo build
```

Note that this will build in debug mode, which is not as performant. 
To get the most speed out of the client, please build in release mode:

```bash
cargo build --release
```

If you want to install the client onto your system (into `$HOME/.cargo/bin`), run:

```bash
cargo install --path client/
```

Alternatively, you can install it directly from git:

```bash
cargo install --git https://github.com/nimiq/core-rs-albatross.git
```

## Usage

After installation, you can run the client directly, like this:

```bash
nimiq-client
```

### Configuration

By default the client will look for a configuration file in `$HOME/.nimiq/client.toml`. 
In order to create this file yourself, you can use the example config file as follow:

```bash
nimiq-client                                                   # Run the client. This will create the example config file.
cp $HOME/.nimiq/client.example.toml $HOME/.nimiq/client.toml   # Create your config from the example.
nano $HOME/.nimiq/client.toml                                  # Edit the config. Explanations are included in the file.
```

If you want to direcly specify your own configuration file when running the client, you can do so as follow:

```bash
nimiq-client -c path/to/client.toml
```

Please take a look at the [`client.example.toml`](lib/src/config/config_file/client.example.toml) for all the configuration options.

### Devnet

The development network is currently in release-candidate phase where we are giving RPC access to interested developers.
Its main purpose is to invite all developers to exercise and test the Nimiq 2.0 RPC functionality (to see how it fits for
their app use cases), and we invite them to file and report any [issues](https://github.com/nimiq/core-rs-albatross/issues/new)
through our GitHub repository.

[ARPL](https://github.com/sisou/arpl) is the recommended command line tool to connect to the devnet RPC, like this:

```
arpl -u "https://seed1.v2.nimiq-testnet.com:8648/?secret=<TOKEN>" status
```

Using the TOKEN that will be given after requesting developer access from a team member through our social media channels.

And get funds to experiment with it (for example, by becoming a validator) using the [Devnet Faucet](http://faucet.v2.nimiq-testnet.com/):

```
curl -X POST -H "Content-Type: application/x-www-form-urlencoded" -d "address=NQXX XXXX XXXX XXXX XXXX XXXX XXXX XXXX XXXX"  http://faucet.v2.nimiq-testnet.com/tapit
```

## Contributing

If you'd like to contribute to the development of Nimiq please follow our [Code of Conduct](/.github/CODE_OF_CONDUCT.md)
and [Contributing Guidelines](/.github/CONTRIBUTING.md).

Small note: When editing the README, please conform to the [standard-readme](https://github.com/RichardLitt/standard-readme) specification.

## License

This project is licensed under the [Apache License 2.0](./LICENSE.md).
