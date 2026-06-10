use std::cmp;

use nimiq_block::Block;
use nimiq_blockchain_interface::{
    AbstractBlockchain, BlockchainEvent, ChainInfo, PushError, PushResult,
};
use nimiq_database::traits::{ReadTransaction, WriteTransaction};
use nimiq_primitives::policy::Policy;
use parking_lot::{RwLockUpgradableReadGuard, RwLockWriteGuard};

use crate::{interface::HistoryInterface, Blockchain};

/// Implements methods to sync a full node via macro blocks.
impl Blockchain {
    /// Pushes an election block backwards into the chain.
    /// This is used to update the `previous_slots` if needed.
    pub fn update_previous_slots(
        this: RwLockUpgradableReadGuard<Self>,
        block: Block,
    ) -> Result<PushResult, PushError> {
        // Must be a macro block.
        assert!(block.is_election());

        if this.state.previous_slots.is_some() {
            return Ok(PushResult::Ignored);
        }

        let read_txn = this.read_transaction();

        // Check if we already know this block.
        if this
            .chain_store
            .get_chain_info(&block.hash(), false, Some(&read_txn))
            .is_ok()
        {
            return Ok(PushResult::Known);
        }

        // Perform block intrinsic checks.
        let max_timestamp = this.now().saturating_add(Policy::TIMESTAMP_MAX_DRIFT);
        block.verify(this.network_id, max_timestamp)?;

        // Check if we have this block's successor.
        let prev_block = &this.state.election_head;

        // Verify that the block is indeed the predecessor.
        if block.hash() != prev_block.header.parent_election_hash {
            return Err(PushError::InvalidPredecessor);
        }

        read_txn.close();

        // Upgrade the blockchain lock
        let mut this = RwLockUpgradableReadGuard::upgrade(this);

        // Store the election block header.
        if let Block::Macro(ref macro_block) = block {
            this.state.previous_slots = macro_block.get_validators();
        }

        debug!(
            block_number = %this.state.main_chain.head.block_number(),
            kind = "update_previous_slots",
            "Updated previous slots",
        );

        Ok(PushResult::Extended)
    }

    /// Pushes a macro block into the blockchain. This is used when we have already synced to the
    /// most recent election block and now need to push a checkpoint block.
    /// But this function is general enough to allow pushing any macro block (checkpoint or election)
    /// at any state of the node (synced, partially synced, not synced).
    pub fn push_macro(
        this: RwLockUpgradableReadGuard<Self>,
        block: Block,
    ) -> Result<PushResult, PushError> {
        // Must be a macro block.
        assert!(block.is_macro());

        let read_txn = this.read_transaction();

        // Check if we already know this block.
        if this
            .chain_store
            .get_chain_info(&block.hash(), false, Some(&read_txn))
            .is_ok()
        {
            return Ok(PushResult::Known);
        }

        if block.block_number() <= this.state.macro_info.head.block_number() {
            return Ok(PushResult::Ignored);
        }

        // Perform block intrinsic checks.
        let max_timestamp = this.now().saturating_add(Policy::TIMESTAMP_MAX_DRIFT);
        block.verify(this.network_id, max_timestamp)?;

        // Verify that the block is a valid macro successor to our current macro head.
        block.verify_macro_successor(this.state.macro_info.head.unwrap_macro_ref())?;

        // Verify that the block is valid for the current validators.
        block.verify_validators(this.current_validators().unwrap())?;

        // At this point we know that the block is correct. We just have to push it.
        let block_hash = block.hash();
        let block_number = block.block_number();
        let is_election_block = Policy::is_election_block_at(block_number);
        let is_protocol_upgrade =
            is_election_block && block.version() > this.state.current_version();
        let chain_info = ChainInfo::new(block, true);

        read_txn.close();
        let mut txn = this.write_transaction();

        this.state
            .accounts
            .reinitialize_as_incomplete(&mut (&mut txn).into());

        this.chain_store
            .put_chain_info(&mut txn, &block_hash, &chain_info, true);
        this.chain_store.set_head(&mut txn, &block_hash);

        // Since it's a macro block, we have to clear the ChainStore.
        if is_election_block {
            let max_epochs_stored =
                cmp::max(this.config.max_epochs_stored, Policy::MIN_EPOCHS_STORED);

            // Calculate the epoch to be pruned.
            let pruned_epoch = Policy::epoch_at(block_number).checked_sub(max_epochs_stored);

            if let Some(pruned_epoch) = pruned_epoch {
                // Prune the Chain Store.
                this.chain_store.prune_epoch(pruned_epoch, &mut txn);

                if !this.config.keep_history {
                    // Prune the History Store.
                    // We will never prune pre-genesis data here.
                    if pruned_epoch > 0 {
                        this.history_store.remove_history(&mut txn, pruned_epoch);
                    }
                }
            }
        }

        txn.commit();

        // Upgrade the lock as late as possible.
        let mut this = RwLockUpgradableReadGuard::upgrade(this);

        if let Block::Macro(ref macro_block) = chain_info.head {
            this.state.macro_info = chain_info.clone();
            this.state.macro_head_hash = block_hash.clone();

            if is_election_block {
                // Verify the hardcoded checkpoint (if any) against our own chain.
                nimiq_genesis::verify_election_checkpoint(
                    this.network_id,
                    macro_block.block_number(),
                    &block_hash,
                );

                this.state.election_head = macro_block.clone();
                this.state.election_head_hash = block_hash.clone();

                let old_slots = this.state.current_slots.take().unwrap();
                this.state.previous_slots.replace(old_slots);

                let new_slots = macro_block.get_validators().unwrap();
                this.state.current_slots.replace(new_slots);
            }
        }

        this.state.main_chain = chain_info;
        this.state.head_hash = block_hash.clone();

        // Downgrade the lock again as the notify listeners might want to acquire read access themselves.
        let this = RwLockWriteGuard::downgrade_to_upgradable(this);

        #[cfg(feature = "metrics")]
        this.metrics.note_extend(&this.state.main_chain.head);
        debug!(
            block = %this.state.main_chain.head,
            num_transactions = this.state.main_chain.head.num_transactions(),
            kind = "push_macro",
            "Accepted block",
        );

        // We shouldn't log errors if there are no listeners.
        this.notifier
            .send(BlockchainEvent::Extended(block_hash.clone()))
            .ok();

        if is_election_block {
            this.notifier
                .send(BlockchainEvent::EpochFinalized(block_hash.clone()))
                .ok();
            if is_protocol_upgrade {
                this.notifier
                    .send(BlockchainEvent::ProtocolUpgrade(
                        block_hash,
                        this.state.current_version(),
                    ))
                    .ok();
            }
        } else {
            this.notifier
                .send(BlockchainEvent::Finalized(block_hash))
                .ok();
        }

        // We don't have any block logs, so we do not notify the block log stream.

        Ok(PushResult::Extended)
    }
}
