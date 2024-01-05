# Guide to become a validator using JSONRPC-CLI

## Table of contents

1. Become a Validator
2. Stake on Behalf of a Validator
3. Inactivate and Remove stake
4. Other useful commands

**Note**

For consistency purposes, we display the data types commands as an example, such `./jsonrpc-cli 127.0.0.1:9100 getValidatorByAddress '{"address": "<address>"}’`. The accurate data should look like the following:

- `'{"address": "NQ16 LEK4 D4AE 80YS DS1N Q7PK 63PU AYVB GGVF"}'`
- `'{"wallet": "NQ16 LEK4 D4AE 80YS DS1N Q7PK 63PU AYVB GGVF"}'`
- `'{"signing_secret_key": "88791c723c7bb4ce5701a2e6b9ad93930e1800d72dddeb054e6d8a74ae50ceef"}'`
- `'{"voting_secret_key": "cceb56e6248800b690d4e1e0caa70fff5b2c0af8ea5cb1cea1a8ba26a574432eee8996e0ba3fc895447bab5445890731fb9bb8182bc4cfe97f43f774798aae4582c26549aa562bea5b6d123cc5ab778d5f0b3c1c91b7acd2d4f6a85b676f0000"}'`
- `'{"key_data": "8f7f4725b5b4b730f12cdd7d348ad4eb6157890ff689c70a1d0eba970c04db93"}'`
- `'{"validity_start_height": "6789"}'`
- `'{"value": 100}'`
- `'{"fee": 2}'`

Value and fees are returned in Luna.

## 1. Become a Validator

To become a validator, you need to have an account with at least the validator deposit fee (10 000 NIM).
This guide assumes that this condition is met and that the client that this RPC commands will be run for is already configured to validate (check [this guide](becoming_validator.md#22-configure) to setup you client).

These are the steps to becoming a validator using `jsonrpc-cli`:

### 1.1 Get your Validator Key and Proof of Knowledge

1. Use the following command to get the validator address:

```
./jsonrpc-cli 127.0.0.1:9100 getAddress '{"null": []}'
```

2. Use the response to get the data of your validator with the following command:

```
./jsonrpc-cli 127.0.0.1:9100 getValidatorByAddress '{"address": "<address>"}'
```

### 1.2 Unlock your Account

To be able to sign transactions, you need to *unlock* your Account in your node:

```
./jsonrpc-cli 127.0.0.1:9100 unlockAccount '{"address": "<address>"}'
# or
./jsonrpc-cli 127.0.0.1:9100 unlockAccount '{"address": "<address>", "passphrase": "MyPassphrase"}'
```

### 1.3 Send Validator Transaction

Finally, we become a validator:

```
./jsonrpc-cli 127.0.0.1:9100 sendNewValidatorTransaction '{"sender_wallet": "<address>", "validator_wallet":"<address>", "signing_secret_key": "<SchnorrPublicKey>", "voting_secret_key": "<BlsPublicKey>", "reward_address": "<address>", "signal_data": "", "fee": Coin, "validity_start_height": "u32"}'
```

After you send this command and the transaction gets accepted (the command returns the transaction hash), you will become a validator in the next epoch. Your node will log that it produces blocks, and your account balance will increase when you get the block rewards in macro blocks.

## 2. Stake on Behalf of a Validator

To stake on behalf of a validator, use the following command to send a new staker transaction:

```
./jsonrpc-cli 127.0.0.1:9100 sendNewStakerTransaction '{"sender_wallet": "<address>", "staker_wallet":"<address>", "value": Coin, "fee": Coin, "validity_start_height": "u32"}'
# or
./jsonrpc-cli 127.0.0.1:9100 sendNewStakerTransaction '{"sender_wallet": "<address>", "staker_wallet":"<address>", "delegation": "<address>", "value": Coin, "fee": Coin, "validity_start_height": "u32"}'
```

## 3. Inactivate and Remove stake

### 3.1 Inactivate your Validator

Inactivates the validator via RPC.

```
./jsonrpc-cli 127.0.0.1:9100 sendInactivateValidatorTransaction '{"sender_wallet": "<address>", "validator_address":"<address>", "signing_secret_key": "<SchnorrPublicKey>", "fee": 2, "validity_start_height": "6789"}’
```

### 3.2 Remove delegated Stake

Removes stake from a staker’s address via RPC.

```
./jsonrpc-cli 127.0.0.1:9100 sendRemoveStakeTransaction '{"staker_wallet": "<address>", "recipient": "<address>", "value": Coin, "fee": Coin, "validity_start_height": "u32"}’
```

## 4. Other useful commands

### 4.1 Send a Transaction

To send a transaction, you can use the `sendBasicTransaction` method:

To send a transaction, you need to pass a JSON object as the first argument containing the transaction info:

```
./jsonrpc-cli 127.0.0.1:9100 sendBasicTransaction '{"wallet":"<address>", "recipient": "<address>", "value": Coin, "fee": Coin, "validity_start_height": "u32"}'
```

**Note:** To send a transaction, your sender account must be unlocked in your node (see [this section](#12-unlock-your-account)).

> If you only want to create a transaction but not send it, you can use the `createBasicTransaction` method with the same JSON object as the argument. The method returns the signed transaction as a HEX string. You can then send the transaction with the `sendRawTransaction` method, passing the HEX string as the only argument.
> 
