# Nimiq Proof-of-Work Migration

This repository contains a tool for the migration process from Nimiq Proof-of-Work
to Nimiq Proof-of-Stake (PoS).
The intended users of this utility are nodes who want to become the first validators
in the Nimiq PoS chain or spectator nodes that want to check and reproduce the migration
process.
The functionality in this repository includes:

- Proof-of-Stake Genesis builder: This tool is used to create the Nimiq PoS
  Genesis block, which will include the account state (balances) from the current
  Nimiq PoW chain.
- Validator readiness: Sends a validator ready transaction to signal that the validator
  is ready to start the migration process and also monitors the PoW chain for other
  validators readiness signal. When enough validators are ready, the PoS client can
  start.
- PoS history builder: Migrates the Nimiq PoW history into Nimiq PoS. This is a requisite
  for the genesis block since its history root is calculated from this history.
- Nimiq PoS state builder: Migrates the Nimiq PoW state into Nimiq PoS. This is a
  requisite for the genesis block since its accounts tree root is calculated from
  this state.
- Migration binary: Starts the Nimiq PoS client when enough validators are ready
  in the expected block window.

There are three well defined phases of the migration process:

- Validator Registration: This is the first phase, which is fixed in time and during
  this phase only validator registration transactions are considered. This phase
  is delimited by the Validation-Start and Validation-End blocks. By the end of this
  phase we obtain a list of registered validators for the PoS chain.
- Pre-stake: This phase is delimited by the PreStake-Start and PreStake-End blocks.
  During this phase only pre-stake transactions are considered. By the end of this
  phase, we would know the exact stake distribution of the registered validators.
- Activation: This phase starts with a candidate transition block for PoS and ends
  when at least 80% of validators signals ready for a candidate transition block.
  During this phase only transactions that signal the validator readiness are considered.
  This phase ends when we have enough Validators (80% in terms of stake) ready.
  During the activation period, validators have certain amount of blocks to signal
  readiness for a candidate transition block. If not enough validators signal their
  readiness, a new candidate transition block is picked and validators need to signal
  their readiness for this other candidate block. This process will repeat until reaching
  the 80% threshold mentioned above.
