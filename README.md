# Nimiq Core implementation in Rust _(core-rs)_

![nimiq](docs/nimiq_logo_rgb_horizontal.png)

> Rust implementation of the Nimiq Blockchain Core

[![Build Status](https://travis-ci.com/nimiq/core-rs-albatross.svg?branch=albatross)](https://travis-ci.com/nimiq/core-rs-albatross)
[![dependency status](https://deps.rs/repo/github/nimiq/core-rs-albatross/status.svg)](https://deps.rs/repo/github/nimiq/core-rs-albatross)

**[Nimiq](https://nimiq.com/)**  is a frictionless payment protocol for the web.

This repository is currently in release-candidate phase. If you need a proven stable release to run in a production environment, please use the [JavaScript implementation](https://github.com/nimiq/core-js/) instead. Only use this if you can tolerate bugs and want to help with testing the Nimiq Rust implementation.

The Nimiq Rust client comes without wallet and can currently not be used to send transactions. As a back-bone node it is more performant than the JavaScript implementation though.

## Table of Contents

- [Background](#background)
- [Install](#install)
- [Usage](#usage)
- [Contributing](#contributing)
- [License](#license)

## Background

- [Nimiq White Paper](https://www.nimiq.com/whitepaper/): General information about the Nimiq project.
- [Nimiq Developer Reference](https://nimiq-network.github.io/developer-reference/): Details of the protocol architecture.


## Install

Besides [Rust nightly](https://www.rust-lang.org/learn/get-started#installing-rust) itself, the following packages are required to be able to compile this source code:

- `gcc`
- `pkg-config`
- `libssl-dev` (in Debian/Ubuntu) or `openssl-devel` (in Fedora/Red Hat)

### From crates.io

To download from [crates.io](https://crates.io), compile and install the client:

```bash
cargo +nightly install nimiq-client
```

The binary will be installed in your Cargo directory, which is usually at `$HOME/.cargo/bin`, and should be available in your `$PATH`.

### From Git

Compiling the project is achieved through [`cargo`](https://doc.rust-lang.org/cargo/):

```bash
git clone https://github.com/nimiq/core-rs-albatross
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
cargo +nightly install --git https://github.com/nimiq/core-rs-albatross.git
```


After installing the client you can use it as if you had downloaded it from [crates.io](https://crates.io).

## Usage

After installation, you can run the client directly, like this:

```bash
nimiq-client
```

### Configuration

By default the client will look for a configuration file in `$HOME/.nimiq/client.config`. You need to create this file yourself:

```bash
nimiq-client                                                   # Run the client. This will create the example config file.
cp $HOME/.nimiq/client.example.toml $HOME/.nimiq/client.toml   # Create your config from the example.
nano $HOME/.nimiq/client.toml                                  # Edit the config. Explanations are included in the file.
```

You can also specify your own configuration file:

```bash
nimiq-client -c path/to/client.toml
```

Take a look at [`client/client.example.toml`](client/client.example.toml) for all the configuration options.


## Contributing

If you'd like to contribute to the development of Nimiq please follow our [Code of Conduct](/.github/CODE_OF_CONDUCT.md) and [Contributing Guidelines](/.github/CONTRIBUTING.md).

Small note: If editing the README, please conform to the [standard-readme](https://github.com/RichardLitt/standard-readme) specification.

## License

This project is under the [Apache License 2.0](./LICENSE.md).
