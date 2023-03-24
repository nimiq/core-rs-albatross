# Albatross Alpha testnet

## Table of contents

1. Run the Client
2. Check your Status with JSON-RPC
3. Create or Import your Account
4. Become a Validator
5. Stake on Behalf of a Validator
6. Inactivate and Unstake
7. Send a Transaction
8. RPC Methods


## 1. Setup

This guide assumes the client has been compiled. Check [this guide](../README.md#installation) for more information on installing the code.
Also all of the commands in this guide are using `arpl` for sending JSON-RPC commands. To install it, use `npm`:

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

To generate a Nimiq address or Schnorr keypair, you can use:

```
cargo run --bin nimiq-address
```

To generate a BLS keypair, you can use:

```
cargo run --bin nimiq-bls
```

### 2.2 Configure

You need to copy `~/.nimiq/client.toml.example` to `~/.nimiq/client.toml`. You can leave all configuration options as they are for now to start a basic full node.

To be able to control your node and to stake and validate, you need to enable the JSON-RPC server in your `client.toml`. Check if the `#[rpc-server]` => `[rpc-server]` is uncommented around line 123.

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
#signing_key = "Schnorr Private Key"
#fee_key = "Schnorr Private Key"
#voting_key = "BLS Private Key"
automatic_reactivate = true
```

Replace the validator address with the one generated in [the previous step](#21-generating-your-validator-address-and-keys) and 
then uncomment the `signing_key`, the `fee_key` and the `voting_key` fields. Put the respective private keys that were generated
in [the previous step](#21-generating-your-validator-address-and-keys).

### 2.3 Sync the Blockchain

After you finish your configuration, simply run the client from inside the `core-rs-albatross` directory with `cargo run —release —bin nimiq-client`. It will connect to the seed node, then to the other nodes in the network, and start finally syncing to the blockchain. See the next chapter to query your node for its status.

## 2.4 Check your Status with JSON-RPC

If you enabled the JSON-RPC Server in your node’s configuration, you can query your node and send commands with `arpl`.
To check the status of your client, first open an interactive session using the port configured in the [configuration section](#22-configure):

```
bin/run -u <host> -p <port> repl
```

Once in the session, check the status with:

```
status
```

## 3. Become a Validator

To become a validator, the process is to send a _create_ validator transaction. For that you need to have an account
with at least the validator deposit fee (10 000 NIM). This guide assumes that this amount is present in the validator
address. To check if that is the case, use:

```
account:get <validator_address>
```


This guide assumes that this condition is met and that the client that this RPC commands will be run for is already
configured to validate (check [this guide](#22-configure) to setup you client).


### 3.1 Import your validator address

1. Use the following command to import your validator private key:

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

Finally, we become a validator:

```
validator:new <validator_address> <signing_private_key> <voting_private_key>
```

Where `signing_private_key` is the private key of the Schnorr keypair generated for `signing_key` and `voting_private_key`
is the private key of the BLS keypair generated for `voting_key`. 

## 4. Stake on Behalf of a Validator

You can stake your NIM on behalf of an online validator. Your staked NIM then count towards the validator’s stake, increasing their likelihood of becoming a block producer. Note that all block rewards are received by the validator's reward address, not yours. The arrangement of distributing rewards among stakers is made off-chain.

```
stake:start <staker_address> <validator_address> <value>
```

For the transaction to work, the account opf address `staker_address` must have a minimum of `value` in its balance.
Also note that the value must be provided in Lunas.

## 5. Inactivate and Unstake

### 5.1 Inactivate your Validator

Inactivates the validator via RPC.

```
validator:inactivate <validator_address> <validator_address> <signing_private_key>
```

Where `signing_private_key` is the private key of the Schnorr keypair generated for `signing_key`.

### 5.2 Unstake delegated Stake

Removes stake from a staker’s address via RPC.

```
stake:stop <staker_address> <value>
```

Where `value` is the amount of NIM to unstake in lunas.