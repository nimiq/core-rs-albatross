  <a id="README" href="#README" href="https://github.com/nimiq/core-rs-albatross/blob/albatross/README.md">
    <img src="https://raw.githubusercontent.com/nimiq/developer-center/refs/heads/main/assets/images/logos/albatross-repo-logo.svg" alt="Nimiq PoS Albatross Repository" width="600" />
  </a>
<br/>
<br/>

[![Build Status](https://github.com/nimiq/core-rs-albatross/actions/workflows/build+test.yml/badge.svg?branch=albatross)](https://github.com/nimiq/core-rs-albatross/actions/workflows/build+test.yml?query=branch%3Aalbatross)
[![dependency status](https://deps.rs/repo/github/nimiq/core-rs-albatross/status.svg)](https://deps.rs/repo/github/nimiq/core-rs-albatross)

[Nimiq](https://nimiq.com/) is a frictionless payment protocol for the web.

This repository contains the Rust implementation of the Nimiq Proof-of-Stake protocol based on the Albatross consensus algorithm. It is designed to deliver high performance without sacrificing security. The Mainnet is now fully operational and ready for live transactions. It has been rigorously tested and is ready for production use.

For the Testnet use and more detailed information on how to connect and use the network, please refer to the [Testnet](#testnet) section.

---

### Table of Contents
- [Reference](#reference)
- [Hardware Requirements](#hardware-requirements-per-node-type)
- [Installation](#installation)
- [Configuration](#configuration)
- [History Nodes](#history-nodes)
- [Service Nodes Guides](#service-nodes-guides)
- [Docker](#docker)
- [Testnet](#testnet)
- [Software Integrity and Authenticity](#software-integrity-and-authenticity)
- [Contributing](#contributing)
- [License](#license)

### Reference

- [Nimiq Proof-of-Stake Portal](https://www.nimiq.com/albatross/): General information and high level details of the Nimiq Proof-of-Stake blockchain.
- [Nimiq Albatross White Paper](https://arxiv.org/abs/1903.01589): White paper describing the consensus algorithm used in Nimiq Proof-of-Stake.
- [Nimiq Developer Center](https://www.nimiq.com/developers/): The place for all the developer documentation and protocol design implementation.
- [JSON-RPC Specification](https://www.nimiq.com/developers/build/set-up-your-own-node/rpc-docs/): Documentation for interacting with the network using JSON-RPC.
- [Nimiq Proof-of-Stake Migration Technicalities](https://www.nimiq.com/developers/migration/migration-technical-details): Migration process to Nimiq Proof-of-Stake.
- [Migration for Integrators](https://www.nimiq.com/developers/migration/migration-integrators): A guide for those who want a more in depth overview of the transitioning process from Proof-of-Work to Proof-of-Stake.
- [Blockchain Explorer](https://nimiq.watch/): Block Explorer for the Mainnet.

## Hardware Requirements per Node Type

| PoS Node Type | Memory | CPU | Storage | Network | Syncing Time |
| --- | --- | --- | --- | --- | --- |
| **History** (check additional [instructions](#history-nodes)) | Minimum 16GB RAM (higher recommended) | Minimum 4 vCPUs, 8 recommended | Minimum 1TB of storage (2TB when enabling indexing); storage usage starts at a few gigabytes and grows linearly with blockchain size over time | High-speed, reliable internet connection; Good I/O performance (SSDs required) | Sync time increases over the life of the blockchain |
| **Full** | Minimum 16GB RAM | 4 vCPUs recommended | Minimum Minimum 60GB of storage | High-speed, reliable internet connection; Good I/O performance (SSDs recommended) | Sync time grows linearly but slowly |
| **Light** | Minimum 4GB RAM | 64-bit recommended | Works with minimal storage | Moderate-speed internet connection (1 Mbps or higher) | Syncs in a few seconds |

#### Additional Recommendations:
- File System: Ensure support for sparse files.
- Clock Synchronization: Use a protocol like NTP for accurate block acceptance, which is essential for validators to produce blocks on time.

### Service Nodes Additional Requirements
Nimiq has also two specific node types with specialized roles in maintaining the network security and performing more advanced tasks.

- **Validators** for block production:
    - **PoS Node Type**: Full or History
    - **Memory**: 16GB RAM minimum
    - **CPU**: 4 vCPUs recommended

- **Prover nodes** for zero-knowledge proof generation:
    - **PoS Node Type**: Full or History
    - **Memory**: 64GB RAM minimum
    - **CPU**: 8vCPUs recommended

## Installation

1. Install the latest version of Rust by following  the instructions on the [Rust website](https://www.rust-lang.org/learn/get-started#installing-rust) and following packages to be able to compile the source code:
    - `clang`
    - `cmake`
    - `libssl-dev` (in Debian/Ubuntu) or `openssl-devel` (in Fedora/Red Hat)
    - `pkg-config` 

We currently do not make any guarantees about the minimum supported Rust version to consumers, but we currently test two versions older than the current Rust stable.

2. Clone the core-rs repository and compile the project with `cargo`:
```bash
git clone https://github.com/nimiq/core-rs-albatross
cd core-rs-albatross
cargo build --release
```

3. Install the client onto your system (into `$HOME/.cargo/bin`) with:
```bash
cargo install --path client/
```

Alternatively, you can install it directly from git:

```bash
cargo install --git https://github.com/nimiq/core-rs-albatross.git
```

### Configuration
You need a configuration file to customize your node according to your specific requirements. Follow one of the methods below to create and edit your configuration file.

**Option A**
The configuration file is generated automatically and in a specific location.
1. Generate the configuration file with the following command:
```bash
cargo run --release --bin nimiq-client
```
This generates a sample file and places it in a folder `./nimiq`.
2. Copy the sample configuration file into a new file in the same directory where you will edit it according to your needs:
```bash
cp $HOME/.nimiq/client.example.toml $HOME/.nimiq/client.toml 
```
3. Edit your configuration file following the explanations inside. Refer to the [configuration settings](https://github.com/nimiq/core-rs-albatross/blob/albatross/lib/src/config/config_file/client.example.toml) for guidance.
4. Run the client:
```bash
cargo run --release --bin nimiq-client
```
By default, the client will look for the config file in `$HOME/.nimiq/client.toml`.

**Option B**
Download the example file and manually place it.
1. Copy this [sample configuration file](https://github.com/nimiq/core-rs-albatross/blob/albatross/lib/src/config/config_file/client.example.toml) to your preferred location.
2. Edit the configuration file and adjust settings as needed. Refer to the **sample configuration file** for guidance.
3. Run the client with the specified file:
```bash
cargo run --release --bin nimiq-client -- -c path/to/client.toml
```

Now your client is launched and running.

**Port Configuration**
Ensure that your system allows network traffic through port **8443/tcp**. Open this port in your firewall to allow the node to connect to the network.

### History Nodes
For the first start of your history node, you must set the environment variable `NIMIQ_OVERRIDE_MAINNET_CONFIG` to point to a configuration file. This file can be downloaded from one of the following sources:

- **Nimiq IPFS gateway link**: https://ipfs.nimiq.io/ipfs/QmWcRRRw4FaKRrznMFt6KemAM35uo9QknMkDaeBzTod33R
- **Via torrent**:
    
    ```
    magnet:?xt=urn:btih:566cec0c350fca917cf5abb00c7dbe8c70884306&dn=nimiq-genesis-main-albatross.toml&tr=https%3A%2F%2Ftorrents.nimiq.io%2Fannounce
    ```
    

After downloading the file, run `NIMIQ_OVERRIDE_MAINNET_CONFIG=/path/to/nimiq-genesis-main-albatross.toml cargo run --release --bin nimiq-client` with the actual path to the file.

This process is required **only for the first start** of the history node. For later restarts, neither the environment variable nor the file are needed. You can even delete the file after the initial setup.

### Service Nodes Guides
You can also choose to run a validator or a prover node. Check our guides with the full step-by-step description:
- [Validators](https://www.nimiq.com/developers/build/set-up-your-own-node/becoming-a-validator)
- [Prover nodes](https://www.nimiq.com/developers/build/set-up-your-own-node/prover-node-guide)

## Docker

1. Create a `data` folder in the main directory with `mkdir ~/data`.
2. Pull the latest image from the container registry:
`docker pull ghcr.io/nimiq/core-rs-albatross:latest`.
3. Create a `client.toml` file in `~/data` with `cp ./lib/src/config/config_file/client.example.toml ~/data/client.toml`.
4. Customize the configuration file to match your requirements. Refer to the [sample configuration file](https://github.com/nimiq/core-rs-albatross/blob/albatross/lib/src/config/config_file/client.example.toml) and [configuration settings](#configuration) for guidance.
5. Run the client via Docker:
`docker run -v $(pwd)/data:/home/nimiq/.nimiq -p 8443:8443 -p 8648:8648 -p 9100:9100 --name nimiq-rpc --rm ghcr.io/nimiq/core-rs-albatross:latest`.

**Overview of Exposed Ports**

| Port | Description |
| --- | --- |
| 8443 | Incoming network connections port |
| 8648 | RPC port |
| 9100 | Metrics port |

## Testnet
The Testnet network is publicly available for testing and experimentation. Its main purpose is to invite everyone to exercise and test the Nimiq Proof-of-Stake functionality and we invite people to file and report any [issues](https://github.com/nimiq/core-rs-albatross/issues/new) through our GitHub repository.

You can use the Testnet by setting the [consensus.network](https://github.com/nimiq/core-rs-albatross/blob/a61a230915726261874163e94fdf81ee9c253404/lib/src/config/config_file/client.example.toml#L121) in your configuration file set to `test-albatross`. Additionally uncomment the [network.seed_nodes](https://github.com/nimiq/core-rs-albatross/blob/b8ed402c9096ffb54afea52347b91ab7831e75de/lib/src/config/config_file/client.example.toml#L29) for the Testnet and comment the Mainnet ones.

#### Getting funds

There are two ways of getting funds:

- Using an account in the [Testnet Nimiq Wallet](https://wallet.pos.nimiq-testnet.com/) and requesting funds in the wallet.
- Directly using the [Testnet Faucet](https://faucet.pos.nimiq-testnet.com/):

```
curl -X POST -H "Content-Type: application/x-www-form-urlencoded" -d "address=NQXX XXXX XXXX XXXX XXXX XXXX XXXX XXXX XXXX" https://faucet.pos.nimiq-testnet.com/tapit
```

## Software Integrity and Authenticity
To ensure the software you are running is authentic and has not been tampered with, refer to the [documentation](https://github.com/nimiq/core-rs-albatross/blob/albatross/build/README.md). It provides details on reproducing Nimiq software and verifying software signatures.

## Contributing

If you'd like to contribute to the development of Nimiq please follow our [Code of Conduct](/.github/CODE_OF_CONDUCT.md)
and [Contributing Guidelines](/.github/CONTRIBUTING.md).
Small note: When editing the README, please conform to the [standard-readme](https://github.com/RichardLitt/standard-readme) specification.

## License

This project is licensed under the [Apache License 2.0](./LICENSE.md).