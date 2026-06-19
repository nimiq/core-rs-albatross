# Protocol Upgrade Signaling Dashboard

A small, read-only browser dashboard that shows whether validators are signaling support
for the next protocol upgrade. It displays the two on-chain thresholds — **% of active
stake** and **% of slots** signaling the target version — and an explorable, filterable
list of all validators.

It is a trustless [web client](../) (`@nimiq/core`, WASM) running as a light client in pico
sync mode, so it needs no node or RPC server — just the built WASM in `../dist`.

## How it works

The upgrade activates on-chain only when **both** of these reach
`Policy::UPGRADE_MIN_SUPPORT` (80%), evaluated at macro-block proposal time
(`validator/src/tendermint.rs`):

- `supporting_stake ≥ 80%` of total active stake, and
- `supporting_slots ≥ 80%` of `Policy::SLOTS`.

The dashboard mirrors this exactly:

- **Target version** = current head `version + 1`.
- A validator **supports** the upgrade iff the first two bytes (big-endian) of its
  `signalData` decode to the target version (the JS equivalent of `Policy::supports_upgrade`).
- **Stake** is summed over the staking contract's active validators
  (`client.getAccount(STAKING_CONTRACT).activeValidators`).
- **Slots** are summed over the validators elected for the current epoch
  (`client.getElectedValidators()`, added on this branch — see
  `web-client/src/client/lib.rs`).

This dashboard relies on the locally built WASM, which must include the
`getElectedValidators()` method added on this branch.

## Build & run with Docker (recommended — no host tooling)

Everything runs in containers; you do **not** need Rust, Node or any wasm tooling on your
machine. Run both commands from the **repository root**.

```sh
# 1. Build web-client/dist (one-shot). First run compiles the wasm toolchain + client,
#    so it takes a while; subsequent runs are cached in Docker volumes.
docker compose -f web-client/upgrade-dashboard/docker-compose.yml run --rm build

# 2. Serve it.
docker compose -f web-client/upgrade-dashboard/docker-compose.yml up serve
```

Then open <http://localhost:8000/upgrade-dashboard/>.

The build only rebuilds the `web` target (main + worker wasm) and reuses the committed
launcher, so it never invokes Node. See `Dockerfile` and `docker-compose.yml` here.

## Build & run natively (alternative)

If you do have the wasm toolchain, from the `web-client` directory:

```sh
./scripts/build.sh --only web,types   # build the dist (includes getElectedValidators)
npx serve .                           # then open /upgrade-dashboard/
```

(An HTTP server is required either way — ES modules will not load over `file://`.)

## Network

By default the dashboard connects to the built-in default network (`MainAlbatross`) using
**pico** sync. Query parameters:

- `?network=testalbatross` (or `devalbatross`) — target another network. For non-default
  networks you usually also need seed nodes; set `SEED_NODES` in `dashboard.js`.
- `?sync=light` — use light sync instead of pico.
