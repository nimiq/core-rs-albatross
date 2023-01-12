use nimiq_block::{Block, BlockError, MacroBlock, MacroBody};
use nimiq_blockchain_interface::{AbstractBlockchain, PushError};
use nimiq_database::Transaction as DBTransaction;
use nimiq_hash::{Blake2bHash, Hash};

use crate::blockchain_state::BlockchainState;
use crate::Blockchain;

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
        block.verify(!trusted)?;

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

            let (proposer_slot, _) = self
                .get_slot_owner_at(block.block_number(), offset, Some(txn))
                .map_err(|error| {
                    warn!(%error, %block, reason = "failed to determine block proposer", "Rejecting block");
                    PushError::Orphan
                })?;

            // Verify that the block is valid for the given proposer.
            block.verify_proposer(&proposer_slot.signing_key, predecessor.seed())?;

            // Verify that the block is valid for the current validators.
            block.verify_validators(&self.current_validators().unwrap())?;

            // Verify that the transactions in the block are valid.
            self.verify_transactions(block)?;
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
    /// In the case of a macro block the checks we perform vary a little depending if we provide a
    /// block with a body:
    /// - With body: we check each field of the body against the same field calculated from our
    ///   current state.
    /// - Without body: we construct a body using fields calculated from our current state and
    ///   compare its hash with the body hash in the header. In this case we return the calculated
    ///   body.
    pub fn verify_block_state(
        &self,
        state: &BlockchainState,
        block: &Block,
        txn_opt: Option<&DBTransaction>,
    ) -> Result<Option<MacroBody>, PushError> {
        let accounts = &state.accounts;

        // Use a common read transaction for the whole function if none was given.
        let read_txn;
        let txn_opt = Some(match txn_opt {
            Some(txn) => txn,
            None => {
                read_txn = self.read_transaction();
                &read_txn
            }
        });

        // Verify accounts hash if the tree is complete or changes only happened in the complete part.
        if let Some(accounts_hash) = accounts.get_root_hash(txn_opt) {
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

        // Verify the history root.
        let real_history_root = self
            .history_store
            .get_history_tree_root(block.epoch_number(), txn_opt)
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

        // For macro blocks we have additional checks. We simply construct what the body should be
        // from our own state and then compare it with the body hash in the header.
        if let Block::Macro(macro_block) = block {
            // Get the lost rewards and disabled sets.
            let staking_contract = self.get_staking_contract();

            let real_lost_rewards = staking_contract.previous_lost_rewards();

            let real_disabled_slots = staking_contract.previous_disabled_slots();

            // Get the validators.
            let real_validators = if macro_block.is_election_block() {
                Some(self.next_validators(&macro_block.header.seed))
            } else {
                None
            };

            // Check the real values against the block.
            if let Some(body) = &macro_block.body {
                // If we were given a body, then check each value against the corresponding value in
                // the body.
                if real_lost_rewards != body.lost_reward_set {
                    warn!(
                        %block,
                        reason = "lost rewards set doesn't match real lost rewards set",
                        "Rejecting block"
                    );
                    return Err(PushError::InvalidBlock(BlockError::InvalidValidators));
                }

                if real_disabled_slots != body.disabled_set {
                    warn!(
                        %block,
                        reason = "Disabled set doesn't match real disabled set",
                        "Rejecting block"
                    );
                    return Err(PushError::InvalidBlock(BlockError::InvalidValidators));
                }

                if real_validators != body.validators {
                    warn!(
                        %block,
                        reason = "Validators don't match real validators",
                        "Rejecting block"
                    );
                    return Err(PushError::InvalidBlock(BlockError::InvalidValidators));
                }

                // We don't need to check the nano_zkp_hash here since it was already checked in the
                // `verify_block_body` method.
            } else {
                // If we were not given a body, then we construct a body from our values and check
                // its hash against the block header.
                let real_pk_tree_root = real_validators
                    .as_ref()
                    .and_then(|validators| MacroBlock::pk_tree_root(validators).ok());

                let real_body = MacroBody {
                    validators: real_validators,
                    pk_tree_root: real_pk_tree_root,
                    lost_reward_set: real_lost_rewards,
                    disabled_set: real_disabled_slots,
                };

                let real_body_hash = real_body.hash::<Blake2bHash>();
                if macro_block.header.body_root != real_body_hash {
                    warn!(
                        %block,
                        header_root = %macro_block.header.body_root,
                        body_hash   = %real_body_hash,
                        reason = "Header body hash doesn't match real body hash",
                        "Rejecting block"
                    );
                    return Err(PushError::InvalidBlock(BlockError::BodyHashMismatch));
                }

                // Since we were not given a body, we return the body that we already calculated.
                return Ok(Some(real_body));
            }
        }

        Ok(None)
    }
}
