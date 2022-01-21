use parking_lot::{RwLockUpgradableReadGuard, RwLockWriteGuard};

use nimiq_block::{Block, BlockError, TendermintProof};
use nimiq_database::WriteTransaction;
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_primitives::coin::Coin;
use nimiq_primitives::policy;

use crate::chain_info::ChainInfo;
use crate::history_store::{ExtTxData, ExtendedTransaction, HistoryStore};
use crate::{AbstractBlockchain, Blockchain, BlockchainEvent, PushError, PushResult};
use nimiq_account::{Inherent, InherentType};

/// Implements methods to push macro blocks into the chain when an history node is syncing. This
/// type of syncing is called history syncing. It works by having the node get all the election
/// macro blocks since genesis plus the last macro block (most likely it will be a checkpoint block,
/// but it might be an election block). For these macro blocks the node must also get the
/// corresponding history tree. When the macro blocks are synced, then the node gets all the micro
/// blocks in the current batch and pushes them normally.
/// Note that, when pushing the macro blocks, we rely on the assumption that they were produced by
/// honest validator sets (defined as having less than 1/3 malicious validators). Because of that
/// we don't actually check the validity of the blocks, we just perform the minimal amount of checks
/// necessary to verify that the given block is a successor of our current chain so far and that the
/// corresponding history tree is actually part of the block.
impl Blockchain {
    /// Pushes a macro block (election or checkpoint) into the chain using the history sync method.
    /// You can push election blocks after checkpoint blocks and vice-versa. You can also push macro
    /// blocks even after you pushed micro blocks.
    /// You just cannot push micro blocks with this method.
    pub fn push_history_sync(
        this: RwLockUpgradableReadGuard<Self>,
        block: Block,
        history: &[ExtendedTransaction],
    ) -> Result<PushResult, PushError> {
        // Check that it is a macro block. We can't push micro blocks with this function.
        assert!(
            block.is_macro(),
            "You can't push micro blocks with history sync!"
        );

        // Create a new database read transaction.
        let read_txn = this.read_transaction();

        // Unwrap the block.
        let macro_block = block.unwrap_macro_ref();

        // Check the version
        if macro_block.header.version != policy::VERSION {
            warn!(
                "Rejecting block {} - wrong version ({} != {})",
                block,
                macro_block.header.version,
                policy::VERSION
            );
            return Err(PushError::InvalidBlock(BlockError::UnsupportedVersion));
        }

        // Check if we already know this block.
        if this
            .chain_store
            .get_chain_info(&macro_block.hash(), false, Some(&read_txn))
            .is_some()
        {
            return Ok(PushResult::Known);
        }

        // Get the current macro head.
        let macro_head = &this.state.macro_info.head;

        // Check if we have this block's parent. The checks change depending if the last macro block
        // that we pushed was an election block or not.
        if macro_head.is_election() {
            // We only need to check that the parent election block of this block is the same as our
            // head block.
            if macro_block.header.parent_election_hash != this.state.macro_head_hash {
                warn!(
                    "Rejecting block {} - wrong parent election hash ({} != {})",
                    macro_block,
                    macro_block.header.parent_election_hash,
                    this.state.macro_head_hash,
                );
                return Err(PushError::Orphan);
            }
        } else {
            // We need to check that this block and our head block have the same parent election
            // block and are in the correct order.
            let parent_election_hash = macro_head.parent_election_hash().unwrap();
            if macro_block.header.parent_election_hash != *parent_election_hash {
                warn!(
                    "Rejecting block {} - wrong parent election hash ({} != {})",
                    macro_block, macro_block.header.parent_election_hash, parent_election_hash
                );
                return Err(PushError::Orphan);
            }

            // FIXME Compute the expected block number and compare it to the given one.
            if macro_block.header.block_number <= macro_head.block_number() {
                warn!(
                    "Rejecting block {} - decreasing block number ({} < {})",
                    macro_block,
                    macro_block.header.block_number,
                    macro_head.block_number()
                );
            }
        }

        // Check the history root.
        let history_root = HistoryStore::root_from_ext_txs(history)
            .ok_or(PushError::InvalidBlock(BlockError::InvalidHistoryRoot))?;

        if *block.history_root() != history_root {
            warn!(
                "Rejecting block {} - wrong history root ({} != {})",
                macro_block,
                block.history_root(),
                history_root
            );
            return Err(PushError::InvalidBlock(BlockError::InvalidHistoryRoot));
        }

        // Checks if the body exists.
        let body = macro_block.body.as_ref().ok_or_else(|| {
            warn!("Rejecting block {} - body missing", macro_block);
            PushError::InvalidBlock(BlockError::MissingBody)
        })?;

        // Check the body root.
        let body_hash = body.hash::<Blake2bHash>();
        if macro_block.header.body_root != body_hash {
            warn!(
                "Rejecting block {} - header body hash doesn't match real body hash ({} != {})",
                macro_block, macro_block.header.body_root, body_hash
            );
            return Err(PushError::InvalidBlock(BlockError::BodyHashMismatch));
        }

        // Check the justification.
        if !TendermintProof::verify(macro_block, &this.current_validators().unwrap()) {
            warn!("Rejecting block {} - bad justification", macro_block);
            return Err(PushError::InvalidBlock(BlockError::InvalidJustification));
        }

        drop(read_txn);

        // Extend the chain with this block.
        let prev_macro_info = this.state.macro_info.clone();
        Blockchain::extend_history_sync(this, block, history, prev_macro_info)
    }

    /// Extends the current chain with a macro block (election or checkpoint) during history sync.
    fn extend_history_sync(
        this: RwLockUpgradableReadGuard<Blockchain>,
        block: Block,
        history: &[ExtendedTransaction],
        mut prev_macro_info: ChainInfo,
    ) -> Result<PushResult, PushError> {
        // Create a new database write transaction.
        let mut txn = this.write_transaction();

        // Get the block hash.
        let block_hash = block.hash();

        // Calculate the cumulative transaction fees for the current batch. This is necessary to
        // create the chain info for the block.
        let mut cum_tx_fees = Coin::ZERO;

        let current_batch = policy::batch_at(block.block_number());

        for i in (0..history.len()).rev() {
            if policy::batch_at(history[i].block_number) != current_batch {
                break;
            }

            if let ExtTxData::Basic(tx) = &history[i].data {
                cum_tx_fees += tx.fee;
            }
        }

        // Create the chain info for the current block and store it.
        let chain_info = ChainInfo {
            on_main_chain: true,
            main_chain_successor: None,
            head: block.clone(),
            cum_tx_fees,
        };

        this.chain_store
            .put_chain_info(&mut txn, &block_hash, &chain_info, true);

        // Update the chain info for the previous macro block and store it.
        prev_macro_info.main_chain_successor = Some(chain_info.head.hash());

        this.chain_store.put_chain_info(
            &mut txn,
            &prev_macro_info.head.hash(),
            &prev_macro_info,
            false,
        );

        // Set the head of the chain store to the current block.
        this.chain_store.set_head(&mut txn, &block_hash);

        // We might already know the given epoch partially.
        // Revert our chain to a common ancestor state in case we have adopted a different history.
        // Also skip over any transactions that we already know.
        let first_new_ext_tx = this.revert_to_common_state(&block, history, &mut txn);

        // Separate the extended transactions by block number and type.
        // We know it comes sorted because we already checked it against the history root and
        // extended transactions in the history tree come sorted by block number and type.
        // Ignore the extended transactions that were already added in past macro blocks.
        let mut block_numbers = vec![];
        let mut block_timestamps = vec![];
        let mut block_transactions = vec![];
        let mut block_inherents = vec![];
        let mut prev = 0;

        for ext_tx in history.iter().skip(first_new_ext_tx) {
            if ext_tx.block_number > prev {
                block_numbers.push(ext_tx.block_number);
                block_timestamps.push(ext_tx.block_time);
                block_transactions.push(vec![]);
                block_inherents.push(vec![]);
                prev = ext_tx.block_number;
            }

            match &ext_tx.data {
                ExtTxData::Basic(tx) => block_transactions.last_mut().unwrap().push(tx.clone()),
                ExtTxData::Inherent(tx) => block_inherents.last_mut().unwrap().push(tx.clone()),
            }
        }

        // We go over the blocks one more time and add the FinalizeBatch and FinalizeEpoch inherents
        // to the macro blocks. This is necessary because the History Store doesn't store those inherents
        // so we need to add them again in order to correctly sync.
        for (i, block_number) in block_numbers.iter().enumerate() {
            if policy::is_macro_block_at(*block_number) {
                let finalize_batch = Inherent {
                    ty: InherentType::FinalizeBatch,
                    target: this.staking_contract_address(),
                    value: Coin::ZERO,
                    data: vec![],
                };

                block_inherents.get_mut(i).unwrap().push(finalize_batch);

                if policy::is_election_block_at(*block_number) {
                    let finalize_epoch = Inherent {
                        ty: InherentType::FinalizeEpoch,
                        target: this.staking_contract_address(),
                        value: Coin::ZERO,
                        data: vec![],
                    };

                    block_inherents.get_mut(i).unwrap().push(finalize_epoch);
                }
            }
        }

        // Update the accounts tree, one block at a time.
        for i in 0..block_numbers.len() {
            // Commit block to AccountsTree and create the receipts.
            let receipts = this.state.accounts.commit_batch(
                &mut txn,
                &block_transactions[i],
                &block_inherents[i],
                block_numbers[i],
                block_timestamps[i],
            );

            // Check if the receipts contain an error.
            if let Err(e) = receipts {
                warn!(
                    "Rejecting block {} - commit of block #{} ({} transactions, {} inherents) failed: {:?}",
                    block,
                    block_numbers[i],
                    block_transactions[i].len(),
                    block_inherents[i].len(),
                    e
                );

                txn.abort();
                #[cfg(feature = "metrics")]
                this.metrics.note_invalid_block();
                return Err(PushError::AccountsError(e));
            }
        }
        this.state.accounts.finalize_batch(&mut txn);

        // Unwrap the block.
        let macro_block = block.unwrap_macro_ref();

        // Check the state_root hash against the one in the block.
        let state_root = this.state.accounts.get_root(Some(&txn));
        if macro_block.header.state_root != state_root {
            warn!(
                "Rejecting block {} - header accounts hash doesn't match real accounts hash ({} != {})",
                macro_block,
                macro_block.header.state_root,
                state_root,
            );
            txn.abort();
            #[cfg(feature = "metrics")]
            this.metrics.note_invalid_block();
            return Err(PushError::InvalidBlock(BlockError::AccountsHashMismatch));
        }

        // Macro blocks are final and receipts for the previous batch are no longer necessary
        // as rebranching across this block is not possible.
        this.chain_store.clear_receipts(&mut txn);

        // Store the new extended transactions into the History tree.
        this.history_store.add_to_history(
            &mut txn,
            block.epoch_number(),
            &history[first_new_ext_tx..],
        );

        // Give up database transactions and push lock before creating notifications.
        txn.commit();

        // Update the blockchain state.
        let mut this = RwLockUpgradableReadGuard::upgrade(this);
        this.state.main_chain = chain_info.clone();
        this.state.head_hash = block_hash.clone();
        this.state.macro_info = chain_info;
        this.state.macro_head_hash = block_hash.clone();

        // Check if this block is an election block.
        let is_election_block = macro_block.is_election_block();
        if is_election_block {
            this.state.election_head = macro_block.clone();
            this.state.election_head_hash = block_hash.clone();
            this.state.previous_slots = this.state.current_slots.take();
            this.state.current_slots = macro_block.get_validators();
        }

        let this = RwLockWriteGuard::downgrade_to_upgradable(this);

        debug!(
            "Accepted block {} with {} transactions (history_sync)",
            block,
            block.num_transactions()
        );

        debug!(
            "Accepted epoch #{} with {} items (history_sync)",
            block.epoch_number(),
            history.len()
        );

        if is_election_block {
            this.notifier
                .notify(BlockchainEvent::EpochFinalized(block_hash));
        } else {
            this.notifier.notify(BlockchainEvent::Finalized(block_hash));
        }

        // Return result.
        Ok(PushResult::Extended)
    }

    fn revert_to_common_state(
        &self,
        block: &Block,
        history: &[ExtendedTransaction],
        txn: &mut WriteTransaction,
    ) -> usize {
        // Find the index of the first extended transaction in the current batch.
        let last_macro_block = policy::last_macro_block(self.block_number());
        let mut first_new_ext_tx = history
            .iter()
            .position(|ext_tx| ext_tx.block_number > last_macro_block)
            .unwrap_or(history.len());

        // Check if our adopted non-final history matches the given history.
        // Revert any blocks that don't match.
        let known_history = self
            .history_store
            .get_nonfinal_epoch_transactions(block.epoch_number(), Some(txn));
        if !known_history.is_empty() {
            // Iterate over the known history and the given history in parallel to find the block
            // where the histories diverge (if they do).
            let mut known = known_history.iter();
            let mut given = history.iter().skip(first_new_ext_tx);
            let mut last_known_block = None;
            let diverging_block = loop {
                match (known.next(), given.next()) {
                    (Some(known_tx), Some(given_tx)) => {
                        last_known_block = Some(known_tx.block_number);
                        if *known_tx != *given_tx {
                            break Some(known_tx.block_number);
                        }
                    }
                    (None, Some(given_tx)) => {
                        break match last_known_block {
                            Some(block_number) if block_number == given_tx.block_number => {
                                Some(block_number)
                            }
                            _ => None,
                        };
                    }
                    (Some(known_tx), None) => break Some(known_tx.block_number),
                    (None, None) => break None,
                }
            };

            if let Some(diverging_block) = diverging_block {
                // The histories diverge, so revert our state to the block before the divergence.
                let num_blocks_to_revert = self.block_number() - diverging_block + 1;
                self.revert_blocks(num_blocks_to_revert, txn)
                    .expect("Failed to revert chain");

                // TODO We could incorporate this into the parallel iteration loop above.
                first_new_ext_tx += history
                    .iter()
                    .skip(first_new_ext_tx)
                    .position(|ext_tx| ext_tx.block_number >= diverging_block)
                    .unwrap_or(history.len() - first_new_ext_tx);
            } else {
                // The histories match, so we can skip over all known transactions.
                first_new_ext_tx += known_history.len();
            }
        } else if self.state.main_chain.head.is_micro() {
            // We have micro blocks for the current batch but the history is empty.
            // Check if the given history contains any items before our current block; if so, we
            // need to revert.
            let first_block_number = history[first_new_ext_tx].block_number;
            if first_block_number <= self.block_number() {
                let num_blocks_to_revert = self.block_number() - first_block_number + 1;
                self.revert_blocks(num_blocks_to_revert, txn)
                    .expect("Failed to revert chain");
            }
        };

        first_new_ext_tx
    }

    /// Reverts a given number of micro blocks from the blockchain.
    pub fn revert_blocks(
        &self,
        num_blocks: u32,
        write_txn: &mut WriteTransaction,
    ) -> Result<(), PushError> {
        debug!(
            "Need to revert {} micro blocks from the current epoch",
            &num_blocks
        );
        // Get the chain info for the head of the chain.
        let mut current_info = self
            .get_chain_info(&self.head_hash(), true, Some(write_txn))
            .expect("Couldn't fetch chain info for the head of the chain!");

        // Revert each block individually.
        for _ in 0..num_blocks {
            match current_info.head {
                Block::Micro(ref micro_block) => {
                    // Get the chain info for the parent of the current head of the chain.
                    let prev_info = self
                        .get_chain_info(&micro_block.header.parent_hash, true, Some(write_txn))
                        .expect("Failed to find main chain predecessor while reverting blocks!");

                    // Revert the accounts tree. This also reverts the history store.
                    self.revert_accounts(
                        &self.state.accounts,
                        write_txn,
                        micro_block,
                        prev_info.head.seed().entropy(),
                        prev_info.head.view_number(),
                    )?;

                    current_info = prev_info;
                }
                Block::Macro(_) => {
                    unreachable!();
                }
            }
        }

        Ok(())
    }
}
