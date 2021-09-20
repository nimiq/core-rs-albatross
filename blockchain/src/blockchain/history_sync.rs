use parking_lot::{RwLockUpgradableReadGuard, RwLockWriteGuard};

use nimiq_block::{Block, BlockError};
use nimiq_database::{ReadTransaction, WriteTransaction};
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
        ext_txs: &[ExtendedTransaction],
    ) -> Result<PushResult, PushError> {
        // Check that it is a macro block. We can't push micro blocks with this function.
        assert!(
            block.is_macro(),
            "You can't push micro blocks with history sync!"
        );

        // Create a new database read transaction.
        let read_txn = ReadTransaction::new(&this.env);

        // Unwrap the block.
        let macro_block = block.unwrap_macro_ref();

        // Check the version
        if macro_block.header.version != policy::VERSION {
            warn!("Rejecting block - block with wrong version");
            return Err(PushError::InvalidBlock(BlockError::UnsupportedVersion));
        }

        // Check if we already know this block.
        if this
            .chain_store
            .get_chain_info(&macro_block.hash(), false, Some(&read_txn))
            .is_some()
        {
            warn!("Rejecting block - block already known");
            return Ok(PushResult::Known);
        }

        // Get the chain info of the current macro head.
        let prev_macro_hash = this.macro_head_hash();

        let prev_macro_info = this
            .get_chain_info(&prev_macro_hash, false, Some(&read_txn))
            .expect("Couldn't fetch chain info for the macro head of the chain!");

        // Check if we have this block's parent. The checks change depending if the last macro block
        // that we pushed was an election block or not.
        if policy::is_election_block_at(prev_macro_info.head.block_number()) {
            // We only need to check that the parent election block of this block is the same as our
            // head block.
            if macro_block.header.parent_election_hash != prev_macro_info.head.hash() {
                warn!("Rejecting block - macro block without correct parent");
                return Err(PushError::Orphan);
            }
        } else {
            // We need to check that this block and our head block have the same parent election
            // block and are in the correct order.
            if &macro_block.header.parent_election_hash
                != prev_macro_info.head.parent_election_hash().unwrap()
                || macro_block.header.block_number <= prev_macro_info.head.block_number()
            {
                warn!("Rejecting block - macro block without correct parent");
                return Err(PushError::Orphan);
            }
        }

        // Check the history root.
        let history_root = HistoryStore::get_root_from_ext_txs(ext_txs)
            .ok_or(PushError::InvalidBlock(BlockError::InvalidHistoryRoot))?;

        if block.history_root() != &history_root {
            warn!("Rejecting block - wrong history root");
            return Err(PushError::InvalidBlock(BlockError::InvalidHistoryRoot));
        }

        // Checks if the body exists.
        let body = macro_block
            .body
            .as_ref()
            .ok_or(PushError::InvalidBlock(BlockError::MissingBody))?;

        // Check the body root.
        if body.hash::<Blake2bHash>() != macro_block.header.body_root {
            warn!("Rejecting block - Header body hash doesn't match real body hash");
            return Err(PushError::InvalidBlock(BlockError::BodyHashMismatch));
        }

        // Checks if the justification exists.
        let justification = macro_block
            .justification
            .as_ref()
            .ok_or(PushError::InvalidBlock(BlockError::NoJustification))?;

        // Check the justification.
        if !justification.verify(
            macro_block.hash(),
            macro_block.header.block_number,
            &this.current_validators().unwrap(),
        ) {
            warn!("Rejecting block - macro block with bad justification");
            return Err(PushError::InvalidBlock(BlockError::InvalidJustification));
        }

        info!("Syncing at macro block #{}", block.block_number());

        drop(read_txn);

        // Extend the chain with this block.
        Blockchain::extend_history_sync(this, block, ext_txs, prev_macro_info)
    }

    /// Extends the current chain with a macro block (election or checkpoint) during history sync.
    fn extend_history_sync(
        this: RwLockUpgradableReadGuard<Blockchain>,
        block: Block,
        ext_txs: &[ExtendedTransaction],
        mut prev_macro_info: ChainInfo,
    ) -> Result<PushResult, PushError> {
        let env = this.env.clone();
        // Create a new database write transaction.
        let mut txn = WriteTransaction::new(&env);

        // If there are micro blocks already in the blockchain, then we need to revert the
        // blockchain to the last macro block.
        let num_blocks = this
            .block_number()
            .checked_sub(prev_macro_info.head.block_number())
            .expect("Head of the chain can't be before the macro head!");

        this.revert_blocks(num_blocks, &mut txn)?;

        // Get the block hash.
        let block_hash = block.hash();

        // Calculate the cumulative transaction fees for the current batch. This is necessary to
        // create the chain info for the block.
        let mut cum_tx_fees = Coin::ZERO;

        let current_batch = policy::batch_at(block.block_number());

        for i in (0..ext_txs.len()).rev() {
            if policy::batch_at(ext_txs[i].block_number) != current_batch {
                break;
            }

            if let ExtTxData::Basic(tx) = &ext_txs[i].data {
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

        // Get the index for the first extended transaction that was not already added in past macro
        // blocks.
        let mut first_new_ext_tx = 0;

        for ext_tx in ext_txs {
            if ext_tx.block_number <= prev_macro_info.head.block_number() {
                first_new_ext_tx += 1;
            } else {
                break;
            }
        }

        // Separate the extended transactions by block number and type.
        // We know it comes sorted because we already checked it against the history root and
        // extended transactions in the history tree come sorted by block number and type.
        // Ignore the extended transactions that were already added in past macro blocks.
        let mut block_numbers = vec![];
        let mut block_timestamps = vec![];
        let mut block_transactions = vec![];
        let mut block_inherents = vec![];
        let mut prev = 0;

        for ext_tx in ext_txs.iter().skip(first_new_ext_tx) {
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
            let receipts = this.state.accounts.commit(
                &mut txn,
                &block_transactions[i],
                &block_inherents[i],
                block_numbers[i],
                block_timestamps[i],
            );

            // Check if the receipts contain an error.
            if let Err(e) = receipts {
                warn!("Rejecting block - commit failed: {:?}", e);
                txn.abort();
                #[cfg(feature = "metrics")]
                this.metrics.note_invalid_block();
                return Err(PushError::AccountsError(e));
            }
        }

        // Macro blocks are final and receipts for the previous batch are no longer necessary
        // as rebranching across this block is not possible.
        this.chain_store.clear_receipts(&mut txn);

        // Store the new extended transactions into the History tree.
        this.history_store.add_to_history(
            &mut txn,
            policy::epoch_at(block.block_number()),
            &ext_txs[first_new_ext_tx..],
        );

        // Unwrap the block.
        let macro_block = block.unwrap_macro_ref();

        // Check if this block is an election block.
        let is_election_block = macro_block.is_election_block();

        let mut this = RwLockUpgradableReadGuard::upgrade(this);

        // Update the blockchain state.
        this.state.main_chain = chain_info.clone();
        this.state.head_hash = block_hash.clone();
        this.state.macro_info = chain_info;
        this.state.macro_head_hash = block_hash.clone();

        if is_election_block {
            this.state.election_head = macro_block.clone();
            this.state.election_head_hash = block_hash.clone();
            this.state.previous_slots = this.state.current_slots.take();
            this.state.current_slots = macro_block.get_validators();
        }

        // Give up database transactions and push lock before creating notifications.
        txn.commit();

        let this = RwLockWriteGuard::downgrade(this);

        if is_election_block {
            this.notifier
                .notify(BlockchainEvent::EpochFinalized(block_hash));
        } else {
            this.notifier.notify(BlockchainEvent::Finalized(block_hash));
        }

        // Return result.
        Ok(PushResult::Extended)
    }

    /// Reverts a given number of micro blocks from the blockchain.
    fn revert_blocks(
        &self,
        num_blocks: u32,
        write_txn: &mut WriteTransaction,
    ) -> Result<(), PushError> {
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
