use nimiq_block::{Block, BlockError, BlockHeader};
use nimiq_blockchain_interface::{AbstractBlockchain, PushError};
use nimiq_database::TransactionProxy as DBTransaction;
use nimiq_primitives::policy::Policy;

use crate::{blockchain_state::BlockchainState, Blockchain};

/// Implements methods to verify the validity of blocks.
impl Blockchain {
    /// Verifies a block for the current blockchain state.
    /// This method does a full verification on the block except for the block state checks.
    /// See `verify_block_state` for these type of checks.
    pub fn verify_block(
        &self,
        txn: &DBTransaction,
        block: &Block,
        trusted: bool,
    ) -> Result<(), PushError> {
        // We expect full blocks (with body) here.
        block
            .body()
            .ok_or(PushError::InvalidBlock(BlockError::MissingBody))?;

        // Perform block intrinsic checks.
        block.verify()?;

        // Fetch predecessor block. Fail if it doesn't exist.
        let predecessor = self
            .get_chain_info(block.parent_hash(), false, Some(txn))
            .map(|info| info.head)
            .map_err(|_| PushError::Orphan)?;

        // Verify that the block is a valid immediate successor to its predecessor.
        block.verify_immediate_successor(&predecessor)?;

        // If the block is a macro block, check that it is a valid successor to the current
        // election block.
        if block.is_macro() {
            block.verify_macro_successor(&self.election_head())?;
        }

        // Verify the interlink (or its absence)
        if let BlockHeader::Macro(macro_header) = block.header() {
            if block.is_election() {
                if let Some(interlink) = &macro_header.interlink {
                    let expected_interlink = self.election_head().get_next_interlink().unwrap();

                    if interlink != &expected_interlink {
                        warn!(reason = "Bad Interlink", "Rejecting block");
                        return Err(PushError::InvalidBlock(BlockError::InvalidInterlink));
                    }
                } else {
                    warn!(reason = "Missing Interlink", "Rejecting block");
                    return Err(PushError::InvalidBlock(BlockError::InvalidInterlink));
                }
            }

            if !block.is_election() && macro_header.interlink.is_some() {
                warn!(reason = "Superfluous Interlink", "Rejecting block");
                return Err(PushError::InvalidBlock(BlockError::InvalidInterlink));
            }
        }

        // In trusted don't do slot related checks since they are mostly signature verifications
        // that can be slow.
        if !trusted {
            // Check block for slot and validators
            // Get the intended block proposer.
            let offset = if let Block::Macro(macro_block) = &block {
                macro_block.round()
            } else {
                // Skip and micro block offset is block number
                block.block_number()
            };

            // Get the validator for this round.
            // The blocks predecessor is not necessarily on the main chain, thus the predecessors vrf seed is used.
            let proposer_slot = self
                .get_proposer_at(
                    block.block_number(),
                    offset,
                    predecessor.seed().entropy(),
                    Some(txn),
                )
                .map_err(|error| {
                    warn!(%error, %block, reason = "failed to determine block proposer", "Rejecting block");
                    PushError::Orphan
                })?
                .validator;

            // Verify that the block is valid for the given proposer.
            block.verify_proposer(&proposer_slot.signing_key, predecessor.seed())?;

            // Verify that the block is valid for the current validators.
            block.verify_validators(&self.current_validators().unwrap())?;

            // Verify that the transactions in the block are valid.
            self.verify_transactions(block)?;

            // Verify that the equivocation proofs are valid.
            self.verify_equivocation_proofs(block, txn)?;
        }

        Ok(())
    }

    fn verify_transactions(&self, block: &Block) -> Result<(), BlockError> {
        if let Some(transactions) = block.transactions() {
            for transaction in transactions {
                if !self.tx_verification_cache.is_known(&transaction.hash()) {
                    transaction.get_raw_transaction().verify(self.network_id)?;
                }
            }
        }

        Ok(())
    }

    /// Verifies a block against the blockchain state AFTER it gets updated with the block (ex: checking if
    /// an account has enough funds).
    /// It receives a block as input but that block is only required to have a header (the body and
    /// justification are optional, we don't need them).
    pub fn verify_block_state_post_commit(
        &self,
        state: &BlockchainState,
        block: &Block,
        txn: &DBTransaction,
    ) -> Result<(), PushError> {
        let accounts = &state.accounts;

        // Verify accounts hash if the tree is complete or changes only happened in the complete part.
        if let Some(accounts_hash) = accounts.get_root_hash(Some(txn)) {
            if *block.state_root() != accounts_hash {
                warn!(
                    %block,
                    header_root = %block.state_root(),
                    accounts_root = %accounts_hash,
                    reason = "Header accounts hash doesn't match real accounts hash",
                    "Rejecting block"
                );
                return Err(PushError::InvalidBlock(BlockError::AccountsHashMismatch));
            }
        }

        // Verify the initial punished set for the next batch.
        // It should be equal to the current punished slots after the blockchain state has been updated.
        if let Block::Macro(macro_block) = block {
            // If we don't have the staking contract, there is nothing we can check.
            if let Some(staking_contract) = self.get_staking_contract_if_complete(Some(txn)) {
                let body = macro_block
                    .body
                    .as_ref()
                    .expect("Block body must be present");

                if body.next_batch_initial_punished_set
                    != staking_contract
                        .punished_slots
                        .current_batch_punished_slots()
                {
                    warn!(
                        %macro_block,
                        given_punished_set = ?body.next_batch_initial_punished_set,
                        expected_punished_set = ?staking_contract.punished_slots
                        .current_batch_punished_slots(),
                        reason = "Invalid next batch punished set",
                        "Rejecting block"
                    );
                    return Err(PushError::InvalidBlock(BlockError::InvalidValidators));
                }
            }
        }

        // Verify the history root when we have the full transaction history of the epoch.
        if state.can_verify_history {
            let real_history_root = self
                .history_store
                .get_history_tree_root(block.epoch_number(), Some(txn))
                .ok_or_else(|| {
                    error!(
                        %block,
                        epoch_number = block.epoch_number(),
                        reason = "failed to fetch history tree root for epoch from store",
                        "Rejecting block"
                    );
                    PushError::InvalidBlock(BlockError::InvalidHistoryRoot)
                })?;

            if *block.history_root() != real_history_root {
                warn!(
                    %block,
                    block_root = %block.history_root(),
                    history_root = %real_history_root,
                    reason = "History root doesn't match real history root",
                    "Rejecting block"
                );
                return Err(PushError::InvalidBlock(BlockError::InvalidHistoryRoot));
            }
        }

        Ok(())
    }

    /// Verify that all the given equivocation proofs of a block are actually valid offenses.
    pub fn verify_equivocation_proofs(
        &self,
        block: &Block,
        txn: &DBTransaction,
    ) -> Result<(), PushError> {
        // We don't need to perform any checks if the given block is not a
        // micro block as only micro blocks contain equivocation proofs.
        let micro_block = match block {
            Block::Macro(_) => return Ok(()),
            Block::Micro(micro_block) => micro_block,
        };

        let body = micro_block
            .body
            .as_ref()
            .expect("Block body must be present");

        for equivocation_proof in &body.equivocation_proofs {
            let validators = self
                .get_validators_for_epoch(
                    Policy::epoch_at(equivocation_proof.block_number()),
                    Some(txn),
                )
                .expect("Couldn't calculate validators");
            equivocation_proof.verify(&validators)?;
        }
        Ok(())
    }

    /// Verifies a block against the blockchain state BEFORE changes to the accounts tree and thus to the staking contract.
    /// Some fields in the staking contract are cleared using the FinalizeBatch and FinalizeEpoch Inherents in preparation for the next batch.
    /// Thus, we need to compare the respective fields in the block before clearing the staking contract.
    pub fn verify_block_state_pre_commit(
        &self,
        state: &BlockchainState,
        block: &Block,
        txn: &DBTransaction,
    ) -> Result<(), PushError> {
        // We don't need to perform any checks if the given block is not a macro block.
        let macro_block = match block {
            Block::Macro(macro_block) => macro_block,
            _ => return Ok(()),
        };

        // If we don't have the staking contract, there is nothing we can check.
        let staking_contract = match self.get_staking_contract_if_complete(Some(txn)) {
            Some(staking_contract) => staking_contract,
            None => return Ok(()),
        };

        let body = macro_block
            .body
            .as_ref()
            .expect("Block body must be present");

        // Verify validators.
        let validators = match macro_block.is_election_block() {
            true => Some(self.next_validators(&macro_block.header.seed)),
            false => None,
        };
        if body.validators != validators {
            warn!(
                %macro_block,
                given_validators = ?body.validators,
                expected_validators = ?validators,
                reason = "Invalid validators",
                "Rejecting block"
            );
            return Err(PushError::InvalidBlock(BlockError::InvalidValidators));
        }

        // Verify reward transactions only if we have the complete accounts state as
        // `create_reward_transactions` expects the full state to be present.
        if !state.accounts.is_complete(Some(txn)) {
            return Ok(());
        }

        let reward_transactions =
            self.create_reward_transactions(state, &macro_block.header, &staking_contract);

        if body.transactions != reward_transactions {
            warn!(
                %macro_block,
                given_reward_transactions = ?body.transactions,
                expected_reward_transactions = ?reward_transactions,
                reason = "Invalid reward transactions",
                "Rejecting block"
            );
            return Err(PushError::InvalidBlock(
                BlockError::InvalidRewardTransactions,
            ));
        }

        Ok(())
    }
}
