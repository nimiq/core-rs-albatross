# Nimiq Proof-of-Work Migration

This repository contains a set of tools and utilities to help the migration process from Nimiq Proof-of-Work to Nimiq
Proof-of-Stake (PoS).
The intended users of these utilities are nodes who want to become validators in the Nimiq PoS chain.
The functionality in this repository includes:

- Nimiq Proof-of-Stake Genesis builder: This tool is used to create the Nimiq POS Genesis block, which will include the
  account state (balances) from the current Nimiq POW Chain. It is worth to note that this block would be a continuation
  of the Nimiq POW chain, with the addition of the first validator set and the Nimiq POW balances, i.e.: this block
  would constitute the first election block of the Nimiq PoS chain.
- Validator readiness: Sends a validator ready transaction to signal that the validator is ready to start the migration
  process and also monitors the PoW chain for other validators readiness signal. When enough validators are ready, then
  an election block candidate is automatically elected to be used as the final PoW state to be migrated.
- Nimiq PoS Wrapper: Starts the Nimiq PoS client when the minimum number of block confirmations for the election
  candidate is satisfied.
- Nimiq PoS history builder: Migrates the Nimiq PoW history into Nimiq PoS. This is only necessary for history nodes who
  want to include and support the full history of the Nimiq PoW and PoS chain.
- Nimiq PoS state builder: Migrates the Nimiq PoW state into Nimiq PoS.

There are three well defined phases of the migration process:

- Validator Registration: This is the first phase, which is fixed in time and during this phase only validator registration transactions are considered. This phase is delimited by the Validation-Start and Validation-End blocks. By the end of this phase we obtain a list of registered validators for the PoS chain.
- Pre-stake: This phase starts when the Validator Registration phase ends and it ends with the Activation-Start block. During this phase only pre-stake transactions are considered. By the end of this phase, we would know the exact stake distribution of the registered validators.
- Activation: This phase starts with the Activation-Start block and it ends with the first Election Block of the Nimiq PoS chain. During this phase only transactions that signal the validator readiness are considered. This phase ends when we have enough Validators (2f+1 in terms of stake/slots) ready.
