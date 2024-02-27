# Proof-of-Stake Testnet

## Table of contents

1. Run the Client
2. Check your Status with JSON-RPC
3. Create or Import your Account
4. Become a Validator
5. Stake on Behalf of a Validator
6. Inactivate and Remove Stake
7. Send a Transaction
8. RPC Methods


## 1. Setup

This guide assumes the client has been compiled. Check [this guide](../README.md#installation) for more information on installing the code.
Also all of the commands in this guide are using [arpl](https://github.com/sisou/arpl) for sending JSON-RPC commands. To install it, use `npm`:

```
npm install -g @sisou/albatross-remote
```

Check [this other guide](becoming_validator_jsoncli.md) for sending raw commands using `JSONRPC-CLI`.

## 2. Run and configure the client

### 2.1 Generating your validator address and keys

For running a validator you need the following items:
- A validator address: Nimiq address.
- A signing key: Schnorr key.
- A fee keypair: Schnorr key.
- A voting keypair: BLS key.

Note that we will use them in the following steps to configure your validator. Write down your public and private keys.

To generate the Schnorr keypairs and the validator address, you can use:

```
cargo run --release --bin nimiq-address
```
> **Note**<br>
> Since we will need two different Schnorr keys and a Nimiq address, the command must be run 3 different times and the output must be written down since it will be needed later in this guide.

To generate a BLS keypair, you can use:

```
cargo run --release --bin nimiq-bls
```
> **Note**<br>
> The output must be written down since it will be needed later in this guide.

### 2.2 Configure

You need to copy `~/.nimiq/client.toml.example` to `~/.nimiq/client.toml`. You can leave all configuration options as they are for now to start a basic full node.

To be able to control your node and to stake and validate, you need to enable the JSON-RPC server in your `client.toml`. Please make sure the rpc section `[rpc-server]` in the configuration file is enabled.

Note that you can also configure your node to use `history` as the `sync_mode`. For that, you could change the `consensus` section of your config file to set `sync_mode` like in the following example:

```
[consensus]
sync_mode = "history"
```

The next step is to set up your validator address and keys in the `validator` section:
```
[validator]
validator_address = "NQXX XXXX XXXX XXXX XXXX XXXX XXXX XXXX XXXX"
signing_key_file = "signing_key.dat"
voting_key_file = "voting_key.dat"
fee_key_file = "fee_key.dat"
signing_key = "Schnorr Private Key"
fee_key = "Schnorr Private Key"
voting_key = "BLS Private Key"
automatic_reactivate = true
```

Replace the validator address and keys generated accordingly:
* The voting key in the config file corresponds to the secret key of the `nimiq-bls` command.
* The signing key corresponds to the private key output of the `nimiq-address` command.
* The fee key corresponds to the private key output of the `nimiq-address` command.
* The validator address corresponds to the address output of the `nimiq-address` command.

> **Note**<br>
> As [previously mentioned](#22-configure), if you are creating a new validator from scratch, and you need to generate all those keys, then you will need to use the `nimiq-address` three times and the `nimiq-bls` one time.
> When sending the create validator transaction, as described below, the validator deposit will be paid from the `wallet` associated to the validator address.
> The `fee-key` is used to pay the fees associated to the automatic unpark/reactivate (if enabled).

### 2.3 TLS Certificate

It is strongly recommended to use a TLS certificate, because in order for the nimiq-pos wallet to connect to a validator, it needs to establish a secure connection. In order to maintain a healthy decentralization level within the network, it is advisable for the nimiq-pos wallet to connect to as many diverse validators as possible.

There are different services where a TLS certificate can be obtained, such as [Let's Encrypt](https://letsencrypt.org/).

Once the certificate is obtained, it can be specified in the Network-TLS section within the config file:

```
[network.tls]
private_key = "./my_private_key.pem"
certificates = "./my_certificate.pem"
```
Note: The full chain of certificates necessary to verify the certificate's validity can be specified in the `pem` file. Usually this full chain is provided by the service that is used to create the TLS certificate.

### 2.4 Sync the Blockchain

After you finish your configuration, simply run the client from inside the `core-rs-albatross` directory with `cargo run —release —bin nimiq-client`. It will connect to the seed node, then to the other nodes in the network, and start finally syncing to the blockchain. See the next chapter to query your node for its status.

## 2.4 Check your Status with JSON-RPC

If you enabled the JSON-RPC Server in your node’s configuration, you can query your node and send commands with `arpl`.
To check the status of your client, first open an interactive session using the port configured in the [configuration section](#22-configure):

```
bin/run repl -u ws://<host>:<port>/ws
```

Once in the session, check the status with:

```
status
```

## 3. Become a Validator

To become a validator, you need to send a _create_ validator transaction. For that you need to have an account
with at least the validator deposit fee (100 000 NIM). This guide assumes that this amount is already present in the validator address. To check if that is the case, use:

```
account:get <validator_address>
```

Note that validators are selected to produce blocks every epoch (every election block), so it may take some time for your validator to be elected to produce blocks.

### 3.1 Import your validator address

Use the following command to import your validator private key:

```
account:import <validator_private_key>
```

Note that this `validator_private_key` is the private key of the Schnorr keypair for the validator address.

### 3.2 Unlock your Account

To be able to sign transactions, you need to *unlock* your Account in your node:

```
account:unlock <validator_address>
```


### 3.3 Send Validator Transaction

Finally, to become a validator:

```
validator:new <validator_address> <signing_private_key> <voting_private_key>
```

Where `signing_private_key` is the private key of the Schnorr keypair generated for `signing_key` and `voting_private_key`
is the private key of the BLS keypair generated for `voting_key`. 

## 4. Stake on Behalf of a Validator

You can stake your NIM on behalf of an online validator. Your staked NIM then counts towards the validator’s stake, increasing their likelihood of producing a block. Note that all block rewards are received by the validator's reward address, not yours. The arrangement of distributing rewards among stakers is made off-chain and usually this is handled by a pool operator or the person who is operating the validator itself.

```
stake:start <staker_address> <validator_address> <value>
```

Also note that the value must be provided in Lunas.

## 5. Inactivate and Remove Stake

### 5.1 Inactivate your Validator

Inactivates the validator via RPC.

```
validator:inactivate <validator_address> <validator_address> <signing_private_key>
```


### 5.2 Remove Stake

Removes stake from a staker’s address via RPC.

```
stake:stop <staker_address> <value>
```

Where `value` is the amount of NIM to remove stake in lunas.