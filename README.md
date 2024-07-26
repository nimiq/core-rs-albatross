# Nimiq Proof-of-Stake

![nimiq](https://raw.githubusercontent.com/nimiq/designs/master/logo/RGB/colored/png/nimiq_logo_rgb_horizontal.png)

> Rust implementation of the Nimiq Proof-of-Stake blockchain.

[![Build Status](https://github.com/nimiq/core-rs-albatross/actions/workflows/build+test.yml/badge.svg?branch=albatross)](https://github.com/nimiq/core-rs-albatross/actions/workflows/build+test.yml?query=branch%3Aalbatross)
[![dependency status](https://deps.rs/repo/github/nimiq/core-rs-albatross/status.svg)](https://deps.rs/repo/github/nimiq/core-rs-albatross)

**[Nimiq](https://nimiq.com/)** is a frictionless payment protocol for the web.

This repository is currently under development. It contains the implementation of the Nimiq Proof-of-Stake protocol based on the [Albatross](https://arxiv.org/abs/1903.01589) consensus algorithm.

Nimiq Proof-of-Stake was conceived with performance in mind without sacrificing security.

Currently, the protocol can be exercised in an environment aimed for developers where bugs are expected to happen.
For more detailed information about how to connect and use the testnet network, please refer to the [Testnet](#testnet) section.

## Table of Contents

- [Background](#background)
- [System requirements](#system-requirements)
- [Installation](#installation)
- [Documentation](#documentation)
- [Usage](#usage)
- [Configuration](#configuration)
- [Testnet](#testnet)
- [Docker](#docker)
- [Contributing](#contributing)
- [License](#license)

## Background

- [Nimiq Proof-of-Stake Portal](https://www.nimiq.com/albatross/): General information and high level details of the Nimiq Proof-of-Stake blockchain
- [Nimiq Albatross White Paper](https://arxiv.org/abs/1903.01589): White paper describing the consensus algorithm used in Nimiq Proof-of-Stake
- [Nimiq Proof-of-Stake migration process](https://www.nimiq.com/blog/nimiq-20-albatross-hard-fork-preparations/): Migration process to Nimiq Proof-of-Stake
- [Nimiq 1.0 Developer Reference](https://nimiq-network.github.io/developer-reference/): Details of the protocol architecture.
- [Nimiq 1.0 JavaScript implementation](https://github.com/nimiq/core-js/): Nimiq 1.0 implementation

## System requirements

- 64-bit computing architecture.
- 2 or more CPU cores
- 2GB of available RAM memory
- 5GB of free disk space, an SSD is highly recommended (100GB or more is required for history nodes)
- File systems with sparse file support.
- It is highly recommended to run a clock synchronization protocol such as NTP. This
  is needed to properly accept blocks according to the timestamp and it is especially
  important for validators in order to produce blocks in the expected timestamps.

## Installation

Besides [Rust stable](https://www.rust-lang.org/learn/get-started#installing-rust) itself,
the following packages are required to be able to compile the source code:

- `clang`
- `cmake`
- `libssl-dev` (in Debian/Ubuntu) or `openssl-devel` (in Fedora/Red Hat)
- `pkg-config`

After installing the previous packages, compiling the project is achieved through [`cargo`](https://doc.rust-lang.org/cargo/):

```bash
git clone https://github.com/nimiq/core-rs-albatross
cd core-rs-albatross
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

## Documentation
Extensive documentation explaining how the protocol is built up, the JSON-RPC specification, how to get started building applications on top of Nimiq and more can be found at the [Nimiq Developer Center](https://www.nimiq.com/developers/).

## Software Integrity and Authenticity
You can refer to the [documentation](./build/README.md) to learn more about reproducing Nimiq software for yourself as well as checking software signatures in order to verify the integrity and authenticity of software.

## Usage

After installation, you can run the client directly, like this:

```bash
cargo run --release --bin nimiq-client
```

### Configuration

By default the client will look for a configuration file in `$HOME/.nimiq/client.toml`.
In order to create this file yourself, you can use the example config file as follow:

```bash
cargo run --release --bin nimiq-client                         # Run the client. This will create the example config file.
cp $HOME/.nimiq/client.example.toml $HOME/.nimiq/client.toml   # Create your config from the example.
nano $HOME/.nimiq/client.toml                                  # Edit the config. Explanations are included in the file.
```

If you want to directly specify your own configuration file when running the client, you can do so as follow:

```bash
cargo run --release --bin nimiq-client -- -c path/to/client.toml
```

Please take a look at the [`client.example.toml`](lib/src/config/config_file/client.example.toml) for all the configuration options.

### Testnet

The testnet network is currently in a phase open to the general public to use.
Its main purpose is to invite everyone to exercise and test the Nimiq Proof-of-Stake functionality and we invite people to file and report any [issues](https://github.com/nimiq/core-rs-albatross/issues/new) through our GitHub repository.

#### Getting funds

There are two ways of getting funds:

- Using an account in the [Testnet Nimiq Wallet](https://wallet.pos.nimiq-testnet.com/) and requesting funds in the wallet.
- Directly using the [Devnet Faucet](https://faucet.pos.nimiq-testnet.com/):

```
curl -X POST -H "Content-Type: application/x-www-form-urlencoded" -d "address=NQXX XXXX XXXX XXXX XXXX XXXX XXXX XXXX XXXX" https://faucet.pos.nimiq-testnet.com/tapit
```

#### Becoming a validator

Check [this guide](https://www.nimiq.com/developers/build/set-up-your-own-node/becoming-a-validator) for steps on becoming a validator.

## Docker

Use `docker pull ghcr.io/nimiq/core-rs-albatross:latest` to pull the latest docker image.
Then mount a volume to configure the client:

```
mkdir data
cp ./lib/src/config/config_file/client.example.toml data/client.toml
```

Note that you can modify the client configuration and this is a must if you are running a validator.
For more documentation on the configuration file, check the [configuration](#configuration) section of this guide.

Once the data directory is created and ready, you can run the client with:

```
docker run -v $(pwd)/data:/home/nimiq/.nimiq -p 8443:8443 -p 8648:8648 -p 9100:9100 --name nimiq-rpc --rm ghcr.io/nimiq/core-rs-albatross:latest
```

Overview of exposed ports:
| Port | Description |
|------|--------------|
| 8443 | Incoming connections port |
| 8648 | RPC port |
| 9100 | Metrics port |

## Contributing

If you'd like to contribute to the development of Nimiq please follow our [Code of Conduct](/.github/CODE_OF_CONDUCT.md)
and [Contributing Guidelines](/.github/CONTRIBUTING.md).

Small note: When editing the README, please conform to the [standard-readme](https://github.com/RichardLitt/standard-readme) specification.

## License

This project is licensed under the [Apache License 2.0](./LICENSE.md).
