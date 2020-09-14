use parking_lot::MutexGuard;

use block::{Block, BlockError, MacroBody};
use database::{ReadTransaction, WriteTransaction};
use hash::{Blake2bHash, Hash};
use primitives::policy;
use transaction::Transaction as BlockchainTransaction;
use utils::merkle;

use crate::chain_info::ChainInfo;
use crate::{Blockchain, BlockchainError, BlockchainEvent, PushError, PushResult};

// TODO: This needs to be redone after Pascal finishes the history root code.
impl Blockchain {
    /// Pushes a macro block without requiring the micro blocks of the previous batch.
    pub fn push_isolated_macro_block(
        &self,
        block: Block,
        transactions: &[BlockchainTransaction],
    ) -> Result<PushResult, PushError> {
        // TODO: Deduplicate code as much as possible...
        // Only one push operation at a time.
        let push_lock = self.push_lock.lock();

        // XXX We might want to pass this as argument to this method
        let read_txn = ReadTransaction::new(&self.env);

        let macro_block = if let Block::Macro(ref block) = block {
            if policy::is_election_block_at(block.header.block_number) != block.is_election_block()
            {
                return Err(PushError::InvalidSuccessor);
            } else {
                block
            }
        } else {
            return Err(PushError::InvalidSuccessor);
        };

        // Check if we already know this block.
        let hash: Blake2bHash = block.hash();
        if self
            .chain_store
            .get_chain_info(&hash, false, Some(&read_txn))
            .is_some()
        {
            return Ok(PushResult::Known);
        }

        if macro_block.is_election_block()
            && self.election_head_hash() != macro_block.header.parent_election_hash
        {
            warn!("Rejecting block - does not follow on our current election head");
            return Err(PushError::Orphan);
        }

        // Check if the block's immediate predecessor is part of the chain.
        let prev_info = self
            .chain_store
            .get_chain_info(
                &macro_block.header.parent_election_hash,
                false,
                Some(&read_txn),
            )
            .ok_or_else(|| {
                warn!("Rejecting block - unknown predecessor");
                PushError::Orphan
            })?;

        // Check the block is the correct election block if it is an election block
        if macro_block.is_election_block()
            && policy::election_block_after(prev_info.head.block_number())
                != macro_block.header.block_number
        {
            warn!(
                "Rejecting block - wrong block number ({:?})",
                macro_block.header.block_number
            );
            return Err(PushError::InvalidSuccessor);
        }

        // Check transactions root
        let hashes: Vec<Blake2bHash> = transactions.iter().map(|tx| tx.hash()).collect();
        let transactions_root = merkle::compute_root_from_hashes::<Blake2bHash>(&hashes);
        if macro_block
            .body
            .as_ref()
            .expect("Missing body!")
            .history_root
            != transactions_root
        {
            warn!("Rejecting block - wrong history root");
            return Err(PushError::InvalidBlock(BlockError::InvalidHistoryRoot));
        }

        // Check Macro Justification
        match macro_block.justification {
            None => {
                warn!("Rejecting block - macro block without justification");
                return Err(PushError::InvalidBlock(BlockError::NoJustification));
            }
            Some(ref justification) => {
                if let Err(_) = justification.verify(
                    macro_block.hash(),
                    &self.current_validators(),
                    policy::TWO_THIRD_SLOTS,
                ) {
                    warn!("Rejecting block - macro block with bad justification");
                    return Err(PushError::InvalidBlock(BlockError::NoJustification));
                }
            }
        }

        // The correct construction of the extrinsics is only checked after the block's inherents are applied.

        // The macro extrinsics must not be None for this function.
        if let Some(ref extrinsics) = macro_block.body {
            let extrinsics_hash: Blake2bHash = extrinsics.hash();
            if extrinsics_hash != macro_block.header.body_root {
                warn!(
                    "Rejecting block - Header extrinsics hash doesn't match real extrinsics hash"
                );
                return Err(PushError::InvalidBlock(BlockError::BodyHashMismatch));
            }
        } else {
            return Err(PushError::InvalidBlock(BlockError::MissingBody));
        }

        // Drop read transaction before creating the write transaction.
        drop(read_txn);

        // We don't know the exact distribution between fork proofs and view changes here.
        // But as this block concludes the epoch, it is not of relevance anymore.
        let chain_info = ChainInfo {
            on_main_chain: false,
            main_chain_successor: None,
            head: block,
            cum_tx_fees: transactions.iter().map(|tx| tx.fee).sum(),
        };

        self.extend_isolated_macro(
            chain_info.head.hash(),
            transactions,
            chain_info,
            prev_info,
            push_lock,
        )
    }

    fn extend_isolated_macro(
        &self,
        block_hash: Blake2bHash,
        transactions: &[BlockchainTransaction],
        mut chain_info: ChainInfo,
        mut prev_info: ChainInfo,
        push_lock: MutexGuard<()>,
    ) -> Result<PushResult, PushError> {
        let mut txn = WriteTransaction::new(&self.env);

        let state = self.state.read();

        let macro_block = chain_info.head.unwrap_macro_ref();
        let is_election_block = macro_block.is_election_block();

        // So far, the block is all good.
        // Remove any micro blocks in the current epoch if necessary.
        let blocks_to_revert = self.chain_store.get_blocks_forward(
            &state.macro_head_hash,
            policy::EPOCH_LENGTH - 1,
            true,
            Some(&txn),
        );
        let mut cache_txn = state.transaction_cache.clone();
        for i in (0..blocks_to_revert.len()).rev() {
            let block = &blocks_to_revert[i];
            let micro_block = match block {
                Block::Micro(block) => block,
                _ => {
                    return Err(PushError::BlockchainError(
                        BlockchainError::InconsistentState,
                    ))
                }
            };

            let prev_view_number = if i == 0 {
                state.macro_info.head.view_number()
            } else {
                blocks_to_revert[i - 1].view_number()
            };
            let prev_state_root = if i == 0 {
                &state.macro_info.head.state_root()
            } else {
                blocks_to_revert[i - 1].state_root()
            };

            self.revert_accounts(&state.accounts, &mut txn, micro_block, prev_view_number)?;

            cache_txn.revert_block(block);

            assert_eq!(
                prev_state_root,
                &state.accounts.hash(Some(&txn)),
                "Failed to revert main chain while rebranching - inconsistent state"
            );
        }

        // We cannot verify the slashed set, so we need to trust it here.
        // Also, we verified that macro extrinsics have been set in the corresponding push,
        // thus we can unwrap here.
        let slashed_set = macro_block.body.as_ref().unwrap().disabled_set.clone();
        let current_slashed_set = macro_block.body.as_ref().unwrap().disabled_set.clone();

        // We cannot check the accounts hash yet.
        // Apply transactions and inherents to AccountsTree.
        let _slots = state
            .previous_slots
            .as_ref()
            .expect("Slots for last epoch are missing");
        let mut inherents = vec![];

        // election blocks finalize the previous epoch.
        if is_election_block {
            inherents.append(&mut self.finalize_previous_batch(&state, &macro_block.header));
        }

        // Commit epoch to AccountsTree.
        let receipts = state.accounts.commit(
            &mut txn,
            transactions,
            &inherents,
            macro_block.header.block_number,
            macro_block.header.timestamp,
        );
        if let Err(e) = receipts {
            warn!("Rejecting block - commit failed: {:?}", e);
            txn.abort();
            #[cfg(feature = "metrics")]
            self.metrics.note_invalid_block();
            return Err(PushError::AccountsError(e));
        }

        drop(state);

        // as with all macro blocks they cannot be rebranched over, so receipts are no longer needed.
        self.chain_store.clear_receipts(&mut txn);

        // Only now can we check macro extrinsics.
        if let Block::Macro(ref mut macro_block) = &mut chain_info.head {
            if is_election_block {
                if let Some(ref validators) =
                    macro_block.body.as_ref().expect("Missing body!").validators
                {
                    let slots = self.next_slots(&macro_block.header.seed);
                    if &slots.validator_slots != validators {
                        warn!("Rejecting block - Validators don't match real validators");
                        return Err(PushError::InvalidBlock(BlockError::InvalidValidators));
                    }
                } else {
                    warn!("Rejecting block - Validators missing");
                    return Err(PushError::InvalidBlock(BlockError::InvalidValidators));
                }
            }

            let computed_extrinsics = MacroBody {
                validators: None,
                lost_reward_set: slashed_set,
                disabled_set: current_slashed_set,
                history_root: Default::default(),
            };

            // The extrinsics must exist for isolated macro blocks, so we can unwrap() here.
            let computed_extrinsics_hash: Blake2bHash = computed_extrinsics.hash();
            if computed_extrinsics_hash != macro_block.header.body_root {
                warn!("Rejecting block - Extrinsics hash doesn't match real extrinsics hash");
                return Err(PushError::InvalidBlock(BlockError::BodyHashMismatch));
            }
        }

        chain_info.on_main_chain = true;
        prev_info.main_chain_successor = Some(chain_info.head.hash());

        self.chain_store
            .put_chain_info(&mut txn, &block_hash, &chain_info, true);
        self.chain_store.put_chain_info(
            &mut txn,
            &chain_info.head.parent_hash(),
            &prev_info,
            false,
        );
        self.chain_store.set_head(&mut txn, &block_hash);
        self.chain_store
            .put_epoch_transactions(&mut txn, &block_hash, transactions);

        // Acquire write lock & commit changes.
        let mut state = self.state.write();
        // FIXME: Macro block sync does not preserve transaction replay protection right now.
        // But this is not an issue in the UTXO model.
        //state.transaction_cache.push_block(&chain_info.head);

        if let Block::Macro(ref macro_block) = chain_info.head {
            state.macro_info = chain_info.clone();
            state.macro_head_hash = block_hash.clone();
            if is_election_block {
                state.election_head = macro_block.clone();
                state.election_head_hash = block_hash.clone();

                let slots = state.current_slots.take().unwrap();
                state.previous_slots.replace(slots);

                let slots = macro_block.get_slots().unwrap();
                state.current_slots.replace(slots);
            }
        } else {
            unreachable!("Block is not a macro block");
        }

        state.main_chain = chain_info;
        state.head_hash = block_hash.clone();
        state.transaction_cache = cache_txn;
        txn.commit();

        // Give up lock before notifying.
        drop(state);
        drop(push_lock);

        if is_election_block {
            self.notifier
                .read()
                .notify(BlockchainEvent::EpochFinalized(block_hash));
        } else {
            self.notifier
                .read()
                .notify(BlockchainEvent::Finalized(block_hash));
        }
        Ok(PushResult::Extended)
    }
}
