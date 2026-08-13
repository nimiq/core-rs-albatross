# mktx Tool

`nimiq-mktx` is the offline CLI for building and signing Nimiq transactions locally. It wraps `nimiq-transaction-builder`, accepts signing keys and parameters via flags, and prints the resulting transaction or signature proof as a hex string ready for broadcast.

- Keeps private keys offline.
- Works when broadcasting is delegated to a different tool or operator.

## Build and run

```bash
cargo run --release --bin nimiq-mktx -- <command> [options]
```

Append `--help` to the root command or any subcommand to inspect flags and examples.

## Argument conventions

Required arguments are positional; optional ones are always named flags.

Signal data on `validator update` is a tri-state, spelled as two mutually exclusive flags
so that no combination can be misread:

- neither flag → leave the signal data unchanged
- `--clear-signal-data` → clear it
- `--new-signal-data <hex>` → set it

`validator set-signal-data` always writes the field, so it has no "leave unchanged" case:

- `--signal-data <hex>` → set it
- `--clear-signal-data` → clear it

Exactly one of the two is required. Omitting both is an error rather than an implicit clear,
so a forgotten value can never silently wipe the field.

## Global options

- `-f`, `--fee <Lunas>` defaults to `0`; ensure the sender balance covers value plus fee.
- `-n`, `--network <NetworkId>` defaults to `MainAlbatross`; you can also choose `TestAlbatross` or `DevAlbatross`.
- `-V`, `--validity-start <height>`; set it to the intended validity start block height.

## Subcommands

- `basic`: standard transfers (recipient, value, optional hex data).
- `htlc`: create, redeem (regular, timeout, early), or sign early proofs with selectable hash types.
- `stake`: manage a staker lifecycle: create, add, update, activate, retire, remove.
- `validator`: manage a validator lifecycle: create, update, signal support for a new block version, deactivate, reactivate, retire, delete; including key rotation and signal data.
- `vesting`: create vesting contracts or redeem vesting payouts with schedule parameters.

Successful commands print a hex string → broadcast transactions through RPC or custom tooling.

## Upgrade signaling

Validators signal support for a chain (hard-fork) upgrade through their `signal_data` field.
Since protocol version 2 this is done with the validator's signing (warm) key, so the cold key
stays offline:

- `validator signal-version` — rewrites only the version bytes and preserves the rest of the
  field. This is the command to use for upgrade signaling.
- `validator set-signal-data` — sets or clears the whole field, overwriting any signaled
  version along with it.

Setting the field with the cold key is still possible through `validator update
--new-signal-data`, but it overwrites the whole field and is no longer needed for signaling.

## Security and troubleshooting

- Protect private keys.
- Verify the selected network matches your target chain before broadcasting.
- Use a broadcaster such as `nimiq-rpc-client` to broadcast the transaction.
