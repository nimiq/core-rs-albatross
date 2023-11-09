use std::{cmp, error::Error, ops::Deref};

use nimiq_account::{BlockLog, BlockLogger};
use nimiq_block::{Block, ForkProof, MicroBlock};
use nimiq_blockchain_interface::{
    AbstractBlockchain, BlockchainEvent, ChainInfo, ChainOrdering, ChunksPushError,
    ChunksPushResult, ForkEvent, PushError, PushResult,
};
use nimiq_database::{
    traits::{ReadTransaction, WriteTransaction},
    TransactionProxy, WriteTransactionProxy,
};
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_keys::Address;
use nimiq_primitives::{
    policy::Policy,
    trie::{
        trie_chunk::{TrieChunkPushResult, TrieChunkWithStart},
        trie_diff::TrieDiff,
    },
};
use nimiq_trie::WriteTransactionProxy as TrieWriteTransactionProxy;
use parking_lot::{RwLockUpgradableReadGuard, RwLockWriteGuard};
use tokio::sync::broadcast::Sender as BroadcastSender;

use crate::{blockchain_state::BlockchainState, Blockchain};

fn send_vec(log_notifier: &BroadcastSender<BlockLog>, logs: Vec<BlockLog>) {
    for log in logs {
        // The log notifier is for informational purposes only, thus may have no listeners.
        // Therefore, no error logs should be produced in this case.
        _ = log_notifier.send(log);
    }
}

/// Implements methods to push blocks into the chain. This is used when the node has already synced
/// and is just receiving newly produced blocks. It is also used for the final phase of syncing,
/// when the node is just receiving micro blocks.
impl Blockchain {
    /// Private function to push a block.
    /// Set the trusted flag to true to skip VRF and signature verifications: when the source of the
    /// block can be trusted.
    fn do_push(
        this: RwLockUpgradableReadGuard<Self>,
        block: Block,
        trusted: bool,
        diff: Option<TrieDiff>,
        chunks: Vec<TrieChunkWithStart>,
    ) -> Result<(PushResult, Result<ChunksPushResult, ChunksPushError>), PushError> {
        // Ignore all blocks that precede (or are at the same height) as the most recent accepted
        // macro block.
        let last_macro_block = Policy::last_macro_block(this.block_number());
        if block.block_number() <= last_macro_block {
            debug!(
                block_no = block.block_number(),
                reason = "we have already finalized a later macro block",
                last_macro_block_no = last_macro_block,
                "Ignoring block",
            );
            return Ok((PushResult::Ignored, Ok(ChunksPushResult::EmptyChunks)));
        }

        // TODO: We might want to pass this as argument to this method.
        let read_txn = this.read_transaction();

        // Check if we already know this block.
        if this
            .chain_store
            .get_chain_info(&block.hash(), false, Some(&read_txn))
            .is_ok()
        {
            return Ok((PushResult::Known, Ok(ChunksPushResult::EmptyChunks)));
        }

        // Check if we have this block's parent.
        let prev_info = this
            .chain_store
            .get_chain_info(block.parent_hash(), false, Some(&read_txn))
            .map_err(|error| {
                warn!(
                    %error,
                    %block,
                    reason = "parent block is unknown",
                    parent_block_hash = %block.parent_hash(),
                    "Rejecting block",
                );
                PushError::Orphan
            })?;

        // Verify the block.
        if let Err(e) = this.verify_block(&read_txn, &block, trusted) {
            warn!(%block, reason = "Block verifications failed", "Rejecting block");
            return Err(e);
        }

        // Detect forks in non-skip micro blocks.
        if block.is_micro() && !block.is_skip() {
            let validator = this
                .get_proposer_at(
                    block.block_number(),
                    block.block_number(),
                    prev_info.head.seed().entropy(),
                    Some(&read_txn),
                )
                .expect("Couldn't find slot owner")
                .validator;
            this.detect_forks(&read_txn, block.unwrap_micro_ref(), &validator.address);
        }

        // Calculate chain ordering.
        let chain_order = ChainOrdering::order_chains(
            this.deref(),
            &block,
            &prev_info,
            |hash| this.get_chain_info(hash, false, Some(&read_txn)),
            |height| this.get_block_at(height, false, Some(&read_txn)),
        );
        let prev_missing_range = this.get_missing_accounts_range(Some(&read_txn));

        read_txn.close();

        let chain_info = ChainInfo::from_block(block, &prev_info, prev_missing_range);

        // Extend, rebranch or just store the block depending on the chain ordering.
        let result = match chain_order {
            ChainOrdering::Extend => {
                return Blockchain::extend(
                    this,
                    chain_info.head.hash(),
                    chain_info,
                    prev_info,
                    diff,
                    chunks,
                );
            }
            ChainOrdering::Superior => {
                return Blockchain::rebranch(
                    this,
                    chain_info.head.hash(),
                    chain_info,
                    diff,
                    chunks,
                );
            }
            ChainOrdering::Inferior => {
                debug!(block = %chain_info.head, "Storing block - on inferior chain");
                PushResult::Ignored
            }
            ChainOrdering::Unknown => {
                debug!(block = %chain_info.head, "Storing block - on fork");
                PushResult::Forked
            }
        };

        let mut txn = this.write_transaction();
        this.chain_store
            .put_chain_info(&mut txn, &chain_info.head.hash(), &chain_info, true);
        if let Some(diff) = &diff {
            this.chain_store
                .put_accounts_diff(&mut txn, &chain_info.head.hash(), diff);
        }
        txn.commit();

        Ok((result, Ok(ChunksPushResult::EmptyChunks)))
    }

    // To retain the option of having already taken a lock before this call the self was exchanged.
    // This is a bit ugly but since push does only really need &mut self briefly at the end for the actual write
    // while needing &self for the majority it made sense to use upgradable read instead of self.
    // Note that there can always only ever be at most one RwLockUpgradableRead thus the push calls are also
    // sequentialized by it.
    /// Pushes a block into the chain.
    pub fn push(
        this: RwLockUpgradableReadGuard<Self>,
        block: Block,
    ) -> Result<PushResult, PushError> {
        // The following assert requires fetching the root node from the accounts trie,
        // which is why we opted to only run it in debug builds.
        debug_assert!(
            this.get_missing_accounts_range(None).is_none(),
            "Should call push only for complete tries"
        );
        Self::push_wrapperfn(this, block, false, None, vec![]).map(|res| res.0)
    }

    pub fn push_with_chunks(
        this: RwLockUpgradableReadGuard<Self>,
        block: Block,
        diff: TrieDiff,
        chunks: Vec<TrieChunkWithStart>,
    ) -> Result<(PushResult, Result<ChunksPushResult, ChunksPushError>), PushError> {
        Self::push_wrapperfn(this, block, false, Some(diff), chunks)
    }

    // To retain the option of having already taken a lock before this call the self was exchanged.
    // This is a bit ugly but since push does only really need &mut self briefly at the end for the actual write
    // while needing &self for the majority it made sense to use upgradable read instead of self.
    // Note that there can always only ever be at most one RwLockUpgradableRead thus the push calls are also
    // sequentialized by it.
    /// Pushes a block into the chain.
    /// The trusted version of the push function will skip some verifications that can only be skipped if
    /// the source is trusted. This is the case of a validator pushing its own blocks
    pub fn trusted_push(
        this: RwLockUpgradableReadGuard<Self>,
        block: Block,
    ) -> Result<PushResult, PushError> {
        Self::push_wrapperfn(this, block, true, None, vec![]).map(|res| res.0)
    }

    /// Commits a set of chunks to the blockchain.
    pub fn commit_chunks(
        &self,
        chunks: Vec<TrieChunkWithStart>,
        block_hash: &Blake2bHash,
    ) -> Result<ChunksPushResult, ChunksPushError> {
        let state_root = self.state.main_chain.head.state_root();
        let mut chunk_result = Ok(ChunksPushResult::EmptyChunks);
        let mut chunks_committed = 0;
        let mut chunks_ignored = 0;
        for (i, chunk_data) in chunks.into_iter().enumerate() {
            let mut txn = self.write_transaction();
            log::trace!(
                "Committing chunk for block: {} chunk: {} start_key: {}",
                block_hash,
                chunk_data.chunk,
                chunk_data.start_key
            );
            let result = self.state.accounts.commit_chunk(
                &mut (&mut txn).into(),
                chunk_data.chunk,
                state_root.clone(),
                chunk_data.start_key,
            );
            match result {
                Err(e) => {
                    txn.abort();
                    log::warn!("Commit chunk for block {} failed: {}", block_hash, e,);
                    chunk_result = Err(ChunksPushError::AccountsError(i, e));
                    break;
                }
                Ok(TrieChunkPushResult::Applied) => {
                    chunks_committed += 1;
                    chunk_result = Ok(ChunksPushResult::Chunks(chunks_committed, chunks_ignored));
                }
                Ok(TrieChunkPushResult::Ignored) => {
                    // The chunk has been ignored, but might still have been valid.
                    log::debug!("Commit chunk for block {} was ignored.", block_hash,);
                    chunks_ignored += 1;
                    chunk_result = Ok(ChunksPushResult::Chunks(chunks_committed, chunks_ignored));
                }
            };

            txn.commit();
        }
        chunk_result
    }

    fn push_wrapperfn(
        this: RwLockUpgradableReadGuard<Self>,
        block: Block,
        trust: bool,
        diff: Option<TrieDiff>,
        chunks: Vec<TrieChunkWithStart>,
    ) -> Result<(PushResult, Result<ChunksPushResult, ChunksPushError>), PushError> {
        #[cfg(not(feature = "metrics"))]
        {
            Self::do_push(this, block, trust, diff, chunks)
        }
        #[cfg(feature = "metrics")]
        {
            let metrics = this.metrics.clone();
            let res = Self::do_push(this, block, trust, diff, chunks);
            metrics.note_push_result(&res);
            res
        }
    }

    /// Extends the current main chain.
    fn extend(
        this: RwLockUpgradableReadGuard<Blockchain>,
        block_hash: Blake2bHash,
        mut chain_info: ChainInfo,
        mut prev_info: ChainInfo,
        diff: Option<TrieDiff>,
        chunks: Vec<TrieChunkWithStart>,
    ) -> Result<(PushResult, Result<ChunksPushResult, ChunksPushError>), PushError> {
        let mut txn = this.write_transaction();

        let block_number = this.block_number() + 1;
        let is_macro_block = Policy::is_macro_block_at(block_number);
        let is_election_block = Policy::is_election_block_at(block_number);

        let mut block_logger = BlockLogger::new_applied(
            block_hash.clone(),
            block_number,
            chain_info.head.timestamp(),
        );
        let total_tx_size = this.check_and_commit(
            &this.state,
            &chain_info.head,
            diff,
            &mut txn,
            &mut block_logger,
        )?;

        chain_info.on_main_chain = true;
        chain_info.set_cumulative_hist_tx_size(&prev_info, total_tx_size);
        chain_info.history_tree_len =
            this.history_store
                .total_len_at_epoch(Policy::epoch_at(block_number), Some(&txn)) as u64;
        prev_info.main_chain_successor = Some(chain_info.head.hash());

        this.chain_store
            .put_chain_info(&mut txn, &block_hash, &chain_info, true);
        this.chain_store
            .put_chain_info(&mut txn, chain_info.head.parent_hash(), &prev_info, false);
        this.chain_store.set_head(&mut txn, &block_hash);

        if is_election_block {
            let max_epochs_stored =
                cmp::max(this.config.max_epochs_stored, Policy::MIN_EPOCHS_STORED);

            // Calculate the epoch to be pruned. Saturate at zero.
            let pruned_epoch = Policy::epoch_at(block_number).saturating_sub(max_epochs_stored);

            // Prune the Chain Store.
            this.chain_store.prune_epoch(pruned_epoch, &mut txn);

            if !this.config.keep_history {
                // Prune the History Store.
                this.history_store
                    .remove_history(&mut txn, Policy::epoch_at(block_number).saturating_sub(1));
            }
        }

        txn.commit();

        // Upgrade the lock as late as possible.
        let mut this = RwLockUpgradableReadGuard::upgrade(this);

        if let Block::Macro(ref macro_block) = chain_info.head {
            this.state.macro_info = chain_info.clone();
            this.state.macro_head_hash = block_hash.clone();

            if is_election_block {
                this.state.election_head = macro_block.clone();
                this.state.election_head_hash = block_hash.clone();

                let old_slots = this.state.current_slots.take().unwrap();
                this.state.previous_slots.replace(old_slots);

                let new_slots = macro_block.get_validators().unwrap();
                this.state.current_slots.replace(new_slots);
                this.state.can_verify_history = true;
            }
        }

        this.state.main_chain = chain_info;
        this.state.head_hash = block_hash.clone();

        // Downgrade the lock again as the notify listeners might want to acquire read access themselves.
        let this = RwLockWriteGuard::downgrade_to_upgradable(this);

        // Try to apply any chunks we received.
        let chunk_result = this.commit_chunks(chunks, &block_hash);

        let num_transactions = this.state.main_chain.head.num_transactions();
        #[cfg(feature = "metrics")]
        this.metrics.note_extend(num_transactions);
        debug!(
            block = %this.state.main_chain.head,
            num_transactions,
            kind = "extend",
            "Accepted block",
        );

        // We shouldn't log errors if there are no listeners.
        this.notifier
            .send(BlockchainEvent::Extended(block_hash.clone()))
            .ok();
        if is_election_block {
            this.notifier
                .send(BlockchainEvent::EpochFinalized(block_hash))
                .ok();
        } else if is_macro_block {
            this.notifier
                .send(BlockchainEvent::Finalized(block_hash))
                .ok();
        }

        // The log notifier is for informational purposes only, thus may have no listeners.
        // Therefore, no error logs should be produced in this case.
        this.log_notifier
            .send(block_logger.build(total_tx_size))
            .ok();

        Ok((PushResult::Extended, chunk_result))
    }

    /// Rebranches the current main chain.
    fn rebranch(
        this: RwLockUpgradableReadGuard<Blockchain>,
        block_hash: Blake2bHash,
        chain_info: ChainInfo,
        diff: Option<TrieDiff>,
        chunks: Vec<TrieChunkWithStart>,
    ) -> Result<(PushResult, Result<ChunksPushResult, ChunksPushError>), PushError> {
        let target_block = chain_info.head.header();
        debug!(block = %target_block, "Rebranching");

        // Find the common ancestor between our current main chain and the fork chain.
        // Walk up the fork chain until we find a block that is part of the main chain.
        // Store the chain along the way.
        let read_txn = this.read_transaction();

        let mut fork_chain: Vec<(Blake2bHash, ChainInfo, Option<TrieDiff>)> = vec![];
        let mut current: (Blake2bHash, ChainInfo, Option<TrieDiff>) =
            (block_hash, chain_info, diff);

        while !current.1.on_main_chain {
            let prev_hash = current.1.head.parent_hash().clone();

            let prev_info = this
                .chain_store
                .get_chain_info(&prev_hash, true, Some(&read_txn))
                .expect("Corrupted store: Failed to find fork predecessor while rebranching");

            let prev_diff = this
                .chain_store
                .get_accounts_diff(&prev_hash, Some(&read_txn))
                .ok();

            fork_chain.push(current);

            current = (prev_hash, prev_info, prev_diff);
        }
        read_txn.close();

        debug!(
            block = %target_block,
            common_ancestor = %current.1.head,
            no_blocks_up = fork_chain.len(),
            "Found common ancestor",
        );

        // Revert AccountsTree & TransactionCache to the common ancestor state.
        let mut revert_chain: Vec<(Blake2bHash, ChainInfo)> = vec![];
        let mut ancestor = current;

        // Check if ancestor is in current batch.
        if ancestor.1.head.block_number() < this.state.macro_info.head.block_number() {
            warn!(
                block = %target_block,
                reason = "ancestor block already finalized",
                ancestor_block = %ancestor.1.head,
                "Rejecting block",
            );
            return Err(PushError::InvalidFork);
        }

        let mut write_txn = this.write_transaction();

        let mut current = (this.state.head_hash.clone(), this.state.main_chain.clone());

        let mut block_logs = vec![];

        while current.0 != ancestor.0 {
            let block = current.1.head.clone();
            if block.is_macro() {
                panic!("Trying to rebranch across macro block {block}");
            }

            let prev_hash = block.parent_hash().clone();

            let prev_info = this
                .chain_store
                .get_chain_info(&prev_hash, true, Some(&write_txn))
                .expect("Corrupted store: Failed to find main chain predecessor while rebranching");

            if let Some(ref prev_missing_range) = current.1.prev_missing_range {
                let result = this.state.accounts.revert_chunk(
                    &mut (&mut write_txn).into(),
                    prev_missing_range.start.clone(),
                );

                if let Err(e) = result {
                    // Check if the revert chunk failed.
                    return Err(PushError::AccountsError(e));
                }
            }

            let mut block_logger = BlockLogger::new_reverted(block.hash(), block.block_number());
            let total_tx_size = this.revert_accounts(
                &this.state.accounts,
                &mut (&mut write_txn).into(),
                &block,
                &mut block_logger,
            )?;
            block_logs.push(block_logger.build(total_tx_size));

            // Verify accounts hash if the tree is complete or changes only happened in the complete part.
            if let Some(accounts_hash) = this.state.accounts.get_root_hash(Some(&write_txn)) {
                assert_eq!(
                    prev_info.head.state_root(),
                    &accounts_hash,
                    "Inconsistent state after reverting block {} - {:?}",
                    block,
                    block,
                );
            }

            revert_chain.push(current);

            current = (prev_hash, prev_info);
        }

        // Push each fork block.

        let mut fork_iter = fork_chain.iter().rev();

        while let Some(fork_block) = fork_iter.next() {
            let mut block_logger = BlockLogger::new_applied(
                fork_block.0.clone(),
                fork_block.1.head.block_number(),
                fork_block.1.head.timestamp(),
            );

            match this.check_and_commit(
                &this.state,
                &fork_block.1.head,
                fork_block.2.clone(),
                &mut write_txn,
                &mut block_logger,
            ) {
                Ok(total_tx_size) => block_logs.push(block_logger.build(total_tx_size)),
                Err(e) => {
                    warn!(
                        block = %target_block,
                        reason = "failed to apply fork block while rebranching",
                        fork_block = %fork_block.1.head,
                        error = &e as &dyn Error,
                        "Rejecting block",
                    );
                    write_txn.abort();

                    // Delete invalid fork blocks from store.
                    let mut write_txn = this.write_transaction();
                    for block in vec![fork_block].into_iter().chain(fork_iter) {
                        this.chain_store.remove_chain_info(
                            &mut write_txn,
                            &block.0,
                            block.1.head.block_number(),
                        )
                    }
                    write_txn.commit();

                    return Err(PushError::InvalidFork);
                }
            }
        }

        // Unset onMainChain flag / mainChainSuccessor on the current main chain up to (excluding) the common ancestor.
        for reverted_block in revert_chain.iter_mut() {
            reverted_block.1.on_main_chain = false;
            reverted_block.1.main_chain_successor = None;

            this.chain_store.put_chain_info(
                &mut write_txn,
                &reverted_block.0,
                &reverted_block.1,
                false,
            );
        }

        // Update the mainChainSuccessor of the common ancestor block.
        ancestor.1.main_chain_successor = Some(fork_chain.last().unwrap().0.clone());
        this.chain_store
            .put_chain_info(&mut write_txn, &ancestor.0, &ancestor.1, false);

        // Set onMainChain flag / mainChainSuccessor on the fork.
        for i in (0..fork_chain.len()).rev() {
            let main_chain_successor = if i > 0 {
                Some(fork_chain[i - 1].0.clone())
            } else {
                None
            };

            let fork_block = &mut fork_chain[i];
            fork_block.1.on_main_chain = true;
            fork_block.1.main_chain_successor = main_chain_successor;

            // Include the body of the new block (at position 0).
            this.chain_store
                .put_chain_info(&mut write_txn, &fork_block.0, &fork_block.1, i == 0);
        }

        // Commit transaction & update head.
        let new_head_hash = &fork_chain[0].0;
        let new_head_info = &fork_chain[0].1;
        this.chain_store.set_head(&mut write_txn, new_head_hash);
        write_txn.commit();

        // Upgrade the lock as late as possible.
        let mut this = RwLockUpgradableReadGuard::upgrade(this);

        if let Block::Macro(ref macro_block) = new_head_info.head {
            this.state.macro_info = new_head_info.clone();
            this.state.macro_head_hash = new_head_hash.clone();

            if Policy::is_election_block_at(new_head_info.head.block_number()) {
                this.state.election_head = macro_block.clone();
                this.state.election_head_hash = new_head_hash.clone();

                let old_slots = this.state.current_slots.take().unwrap();
                this.state.previous_slots.replace(old_slots);

                let new_slots = macro_block.get_validators().unwrap();
                this.state.current_slots.replace(new_slots);
            }
        }

        this.state.main_chain = new_head_info.clone();
        this.state.head_hash = new_head_hash.clone();

        // Downgrade the lock again as the notified listeners might want to acquire read themselves.
        let this = RwLockWriteGuard::downgrade_to_upgradable(this);

        // Try to apply any chunks we received.
        let chunk_result = this.commit_chunks(chunks, new_head_hash);

        let mut reverted_blocks = Vec::with_capacity(revert_chain.len());
        for (hash, chain_info) in revert_chain.into_iter().rev() {
            debug!(
                block = %chain_info.head,
                num_transactions = chain_info.head.num_transactions(),
                "Reverted block",
            );
            reverted_blocks.push((hash, chain_info.head));
        }

        let mut adopted_blocks = Vec::with_capacity(fork_chain.len());
        for (hash, chain_info, _) in fork_chain.into_iter().rev() {
            debug!(
                block = %chain_info.head,
                num_transactions = chain_info.head.num_transactions(),
                kind = "rebranch",
                "Accepted block",
            );
            adopted_blocks.push((hash, chain_info.head));
        }

        debug!(
            block = %this.state.main_chain.head,
            num_reverted_blocks = reverted_blocks.len(),
            num_adopted_blocks = adopted_blocks.len(),
            "Rebranched",
        );
        #[cfg(feature = "metrics")]
        this.metrics
            .note_rebranch(&reverted_blocks, &adopted_blocks);

        // We do not log errors if there are no listeners.
        this.notifier
            .send(BlockchainEvent::Rebranched(reverted_blocks, adopted_blocks))
            .ok();
        if this.state.main_chain.head.is_election() {
            this.notifier
                .send(BlockchainEvent::EpochFinalized(
                    this.state.head_hash.clone(),
                ))
                .ok();
        } else if this.state.main_chain.head.is_macro() {
            this.notifier
                .send(BlockchainEvent::Finalized(this.state.head_hash.clone()))
                .ok();
        }

        send_vec(&this.log_notifier, block_logs);

        Ok((PushResult::Rebranched, chunk_result))
    }

    fn check_and_commit(
        &self,
        state: &BlockchainState,
        block: &Block,
        diff: Option<TrieDiff>,
        txn: &mut WriteTransactionProxy,
        block_logger: &mut BlockLogger,
    ) -> Result<u64, PushError> {
        // Check transactions against replay attacks. This is only necessary for micro blocks.
        if block.is_micro() {
            let transactions = block.transactions();

            if let Some(tx_vec) = transactions {
                for transaction in tx_vec {
                    let tx_hash = transaction.get_raw_transaction().hash();
                    if self.contains_tx_in_validity_window(&tx_hash, Some(txn)) {
                        warn!(
                            %block,
                            reason = "transaction already included",
                            transaction_hash = %tx_hash,
                            "Rejecting block",
                        );
                        return Err(PushError::DuplicateTransaction);
                    }
                }
            }
        }

        // Macro blocks: Verify the state against the block before modifying the staking contract.
        // (FinalizeBatch and FinalizeEpoch Inherents clear some fields in preparation for the next epoch.)
        if let Err(e) = self.verify_block_state_pre_commit(state, block, txn) {
            warn!(%block, reason = "bad state", error = &e as &dyn Error, "Rejecting block");
            return Err(e);
        }

        // Commit block to AccountsTree.
        let total_tx_size;
        {
            let is_complete = state.accounts.is_complete(Some(txn));
            let mut txn: TrieWriteTransactionProxy = txn.into();
            if is_complete {
                txn.start_recording();
            }
            total_tx_size = self.commit_accounts(state, block, diff, &mut txn, block_logger).map_err(|e| {
                warn!(%block, reason = "commit failed", error = &e as &dyn Error, "Rejecting block");
                #[cfg(feature = "metrics")]
                self.metrics.note_invalid_block();
                e
            })?;
            if is_complete {
                let recorded_diff = txn.stop_recording().into_forward_diff();
                self.chain_store
                    .put_accounts_diff(txn.raw(), &block.hash(), &recorded_diff);
            }
        }

        // Verify the state against the block.
        if let Err(e) = self.verify_block_state_post_commit(state, block, txn) {
            warn!(%block, reason = "bad state", error = &e as &dyn Error, "Rejecting block");
            return Err(e);
        }

        Ok(total_tx_size)
    }

    fn detect_forks(
        &self,
        txn: &TransactionProxy,
        block: &MicroBlock,
        validator_address: &Address,
    ) {
        assert!(!block.is_skip_block());

        // Check if there are two blocks in the same slot and with the same height. Since we already
        // verified the validator for the current slot, this is enough to check for fork proofs.
        // Note: We don't verify the justifications for the other blocks here, since they had to
        // already be verified in order to be added to the blockchain.
        let mut micro_blocks: Vec<Block> =
            match self
                .chain_store
                .get_blocks_at(block.header.block_number, false, Some(txn))
            {
                Ok(blocks) => blocks,
                Err(_) => vec![],
            };

        // Filter out any skip blocks.
        micro_blocks.retain(|block| block.is_micro() && !block.is_skip());

        for micro_block in micro_blocks.drain(..).map(|block| block.unwrap_micro()) {
            // If there's another micro block set to this block height, which also has the same
            // VrfSeed entropy we notify the fork event.
            if block.header.seed.entropy() == micro_block.header.seed.entropy() {
                // Get the justification for the block. We assume that the
                // validator's signature is valid.
                let justification1 = block
                    .justification
                    .clone()
                    .expect("Missing justification")
                    .unwrap_micro();

                let justification2 = micro_block
                    .justification
                    .expect("Missing justification")
                    .unwrap_micro();

                let proof = ForkProof::new(
                    validator_address.clone(),
                    block.header.clone(),
                    justification1,
                    micro_block.header,
                    justification2,
                );

                // We shouldn't log errors if there are no listeners.
                _ = self.fork_notifier.send(ForkEvent::Detected(proof));
            }
        }
    }
}
