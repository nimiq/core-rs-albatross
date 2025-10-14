# mktx Tool

`nimiq-mktx` is the offline CLI for building and signing Nimiq transactions locally. It wraps `nimiq-transaction-builder`, accepts signing keys and parameters via flags, and prints the resulting transaction or signature proof as a hex string ready for broadcast.

- Keeps private keys offline.
- Works when broadcasting is delegated to a different tool or operator.

## Build and run

```bash
cargo run --bin nimiq-mktx -- <command> [options]
```

Append `--help` to the root command or any subcommand to inspect flags and examples.

## Global options

- `-f`, `--fee <Lunas>` defaults to `0`; ensure the sender balance covers value plus fee.
- `-n`, `--network <NetworkId>` defaults to `MainAlbatross`; you can also choose `TestAlbatross` or `DevAlbatross`.
- `-v`, `--validity-start <height>` defaults to `0`; set it to the intended block height.

## Subcommands

- `basic`: standard transfers (recipient, value, optional hex data).
- `htlc`: create, redeem (regular, timeout, early), or sign early proofs with selectable hash types.
- `stake`: manage a staker lifecycle: create, add, update, activate, retire, remove.
- `validator`: manage a validator lifecycle: create, update, deactivate, reactivate, retire, delete; including key rotation and signal data.
- `vesting`: create vesting contracts or redeem vesting payouts with schedule parameters.

Successful commands print a hex string → broadcast transactions through RPC or custom tooling.

## Security and troubleshooting

- Protect private keys.
- Verify the selected network matches your target chain before broadcasting.
- Use a broadcaster such as `nimiq-rpc-client` to broadcast the transaction.
