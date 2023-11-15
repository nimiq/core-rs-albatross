use nimiq_account::BlockLogger;
use nimiq_block::{Block, BlockError, BlockHeader, MacroBlock, MacroBody};
use nimiq_blockchain_interface::{AbstractBlockchain, ChainInfo, PushError};
use nimiq_database::{
    traits::{ReadTransaction, WriteTransaction},
    TransactionProxy as DBTransaction, WriteTransactionProxy,
};
use nimiq_primitives::policy::Policy;

use crate::{blockchain_state::BlockchainState, BlockProducer, Blockchain};

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
            if self
                .history_store
                .has_equivocation_proof(equivocation_proof.locator(), Some(txn))
            {
                return Err(PushError::EquivocationAlreadyIncluded(
                    equivocation_proof.locator(),
                ));
            }
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

    /// Verifies a `proposed_block`, given the round it is proposed in as well as its valid round if applicable.
    pub fn verify_macro_block_proposal(
        &self,
        proposed_block: MacroBlock,
        round: u32,
        valid_round: Option<u32>,
    ) -> Result<MacroBody, PushError> {
        // If the accounts tree is not complete nothing can be done
        if !self.accounts_complete() {
            // Maybe introduce a proper error here. Is this even needed?
            return Err(PushError::IncompleteAccountsTrie);
        }

        // Get the seed of the preceding block.
        let prev_header = self
            .get_block(&proposed_block.header.parent_hash, false, None)
            .map_err(PushError::BlockchainError)?
            .header();

        // The VRF seed will be signed by the proposer of the original round the proposal was proposed in.
        // The round is specified within the header itself, and it should only ever differ from the round
        // value provided if valid_round = Some(vr). Importantly though, that very vr is not necessarily
        // the same as the round given in the header, as it might not be the first time this proposal is
        // re-proposed.
        if let Some(vr) = valid_round {
            if vr < proposed_block.header.round || vr >= round {
                // A VR only makes sense if it is at least the original round of the proposal,
                // while also not being in the future.
                warn!(
                    valid_round,
                    round,
                    ?proposed_block,
                    "Re-proposing proposal with bogus VR"
                );
                // Reject the proposal.
                return Err(PushError::InvalidBlock(BlockError::InvalidSeed));
            }
        } else {
            // Log a warning if a block is re-proposed but fails to produce the proper vr.
            if proposed_block.header.round != round {
                warn!(
                    valid_round,
                    round,
                    ?proposed_block,
                    "Re-proposing proposal without specifying a VR"
                );
                // Reject the proposal.
                return Err(PushError::InvalidBlock(BlockError::InvalidSeed));
            }
        }

        // Create a read transaction to use in the following queries.
        // It is not necessary for the correct execution and only is created to not create 2 different
        // ones within the 2 function calls making use of it here.
        let txn = self.read_transaction();

        // Get the original proposer of the proposal.
        let proposer = self
            .get_proposer_at(
                proposed_block.block_number(),
                proposed_block.header.round,
                prev_header.seed().entropy(),
                Some(&txn),
            )
            .expect("Couldn't find slot owner!")
            .validator
            .signing_key;

        // Fetch predecessor block. Fail if it doesn't exist.
        let predecessor = self
            .get_chain_info(&proposed_block.header.parent_hash, false, Some(&txn))
            .map_err(PushError::BlockchainError)?;

        // Close the transaction as there is no longer a need for it.
        txn.close();

        // Wrap the macro block to use block agnostic verifications.
        let mut block = Block::Macro(proposed_block);

        // Make sure the header verifies
        if let Err(error) = block.header().verify(false) {
            debug!(%error, %block, "Tendermint - await_proposal: Invalid block header");
            return Err(PushError::InvalidBlock(error));
        }

        // Make sure the block is the actual successor to its predecessor.
        if let Err(error) = block.verify_immediate_successor(&predecessor.head) {
            debug!(%error, %block, "Tendermint - await_proposal: Invalid block header for blockchain head");
            return Err(PushError::InvalidBlock(error));
        }

        // Make sure that the (macro) block is the macro successor to the current macro head of the blockchain.
        if let Err(error) = block.verify_macro_successor(&self.macro_head()) {
            debug!(%error, %block, "Tendermint - await_proposal: Invalid block header for blockchain macro head");
            return Err(PushError::InvalidBlock(error));
        }

        // Make sure the proposer is correct using the predecessor to determine slot ownership.
        if let Err(error) = block.verify_proposer(&proposer, predecessor.head.seed()) {
            debug!(%error, %block, "Tendermint - await_proposal: Invalid block header, VRF seed verification failed");
            return Err(PushError::InvalidBlock(error));
        }

        // Check if the predecessor is on the current main chain.
        if self.head_hash() == predecessor.head.hash() {
            let mut txn = self.write_transaction();
            return self.verify_proposal_state(&mut block, &mut txn);
        }
        self.verify_inferior_chain_macro_block_proposal(&mut block, predecessor)
    }

    /// Given a transaction containing relevant change to the state this function will
    /// take the state and compare it pre and post commit to the block.
    fn verify_proposal_state(
        &self,
        block: &mut Block,
        txn: &mut WriteTransactionProxy,
    ) -> Result<MacroBody, PushError> {
        // First compute the macro body if it was not given as part of the proposed block.
        let body = {
            let macro_block = block.unwrap_macro_ref_mut();

            macro_block
                .body
                .get_or_insert(BlockProducer::next_macro_body(
                    self,
                    &macro_block.header,
                    Some(txn),
                ))
                .clone()
        };

        // Get the blockchain state.
        let state = self.state();

        // Verify macro block state before committing accounts.
        if let Err(error) = self.verify_block_state_pre_commit(state, block, txn) {
            debug!(%error, %block, "Tendermint - await_proposal: Invalid macro block state");
            return Err(error);
        }

        // Update our blockchain state using the received proposal. If we can't update the state, we
        // return a proposal timeout.
        if let Err(error) = self.commit_accounts(
            state,
            block,
            None,
            &mut txn.into(),
            &mut BlockLogger::empty(),
        ) {
            debug!(%error, %block, "Tendermint - await_proposal: Failed to commit accounts");
            return Err(error);
        }

        // Check the validity of the block against our state. If it is invalid, we return a proposal
        // timeout. This also returns the block body that matches the block header
        // (assuming that the block is valid).
        if let Err(error) = self.verify_block_state_post_commit(state, block, txn) {
            log::debug!(%error, %block, "Tendermint - await_proposal: Invalid block state");
            return Err(error);
        }

        Ok(body)
    }

    /// Verifies a proposal given as `block`. The block may contain a precalculated body. If it does not exists,
    /// it will be created during verification.
    ///
    /// For inferior chain proposal verification the state must be reverted to the inferior chain head prior
    /// to calling Blockchain::verify_proposal_state.
    ///
    /// It returns the body to the proposal if verification succeeds, or the error if it does not.
    fn verify_inferior_chain_macro_block_proposal(
        &self,
        block: &mut Block,
        prev_info: ChainInfo,
    ) -> Result<MacroBody, PushError> {
        // Check to see if the trie is incomplete. Proposal verification only happens when actively validating and thus the
        // trie should not be incomplete initially. It might be incomplete while rebranching blocks and those need to
        // be taken into account.
        let prev_missing_range = self.get_missing_accounts_range(None);
        if prev_missing_range.is_some() {
            warn!(
                %block,
                ?prev_missing_range,
                "Verifying proposal while current missing accounts is not none",
            );
        }

        // Create a ChainInfo for the proposed block.
        let chain_info = ChainInfo::from_block(block.clone(), &prev_info, prev_missing_range);

        let read_txn = self.read_transaction();
        // First the common ancestor of the two chains needs to be found.
        let (mut ancestor, mut fork_chain) =
            self.find_common_ancestor(block.hash(), chain_info, None, &read_txn)?;

        read_txn.close();

        // fork_chain includes the macro block itself, which must be removed.
        let _removed_proposed_block = fork_chain.remove(0);

        debug!(
            %block,
            common_ancestor = %ancestor.1.head,
            no_blocks_up = fork_chain.len(),
            "Found common ancestor",
        );

        let mut write_txn = self.write_transaction();
        if let Err(remove_chain) =
            Blockchain::rebranch_to(self, &mut fork_chain, &mut ancestor, &mut write_txn)
        {
            // Failed to apply blocks. All blocks within revert chain must be removed.
            // To do that the txn must be aborted first, as the txn will be comitted and
            // prior changes are unwanted.
            write_txn.abort();

            // Delete invalid fork blocks from store.
            // Create a new write transaction which will be committed.
            let mut write_txn = self.write_transaction();
            for block in remove_chain {
                self.chain_store.remove_chain_info(
                    &mut write_txn,
                    &block.0,
                    block.1.head.block_number(),
                );
            }
            write_txn.commit();

            return Err(PushError::InvalidFork);
        }
        // The state is now prepared contained within `write_txn` to just invoke verify_proposal_state.
        self.verify_proposal_state(block, &mut write_txn)
    }
}
