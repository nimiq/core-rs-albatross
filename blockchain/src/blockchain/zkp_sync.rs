use std::{cmp, path::PathBuf};

use parking_lot::{RwLockUpgradableReadGuard, RwLockWriteGuard};

use nimiq_block::{Block, BlockError};
use nimiq_blockchain_interface::{
    AbstractBlockchain, BlockchainEvent, ChainInfo, PushError, PushResult,
};
use nimiq_nano_zkp::{NanoProof, NanoZKP};
use nimiq_primitives::policy::Policy;

use crate::Blockchain;

/// Implements methods to sync a full node via ZKP.
impl Blockchain {
    /// Syncs using a zero-knowledge proof. It receives an election block and a proof that there is
    /// a valid chain between the genesis block and that block.
    /// This brings the node from the genesis block all the way to the most recent election block.
    /// It is the default way to sync for a full node.
    pub fn push_zkp(
        this: RwLockUpgradableReadGuard<Self>,
        block: Block,
        proof: NanoProof,
    ) -> Result<PushResult, PushError> {
        // Must be an election block.
        assert!(block.is_election());

        // Checks if the body exists.
        block
            .body()
            .ok_or(PushError::InvalidBlock(BlockError::MissingBody))?;

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
        block.verify(false)?;

        // Prepare the inputs to verify the proof.
        let genesis_block = this
            .chain_store
            .get_block_at(0, true, Some(&read_txn))
            .unwrap();

        let initial_header_hash = <[u8; 32]>::from(genesis_block.hash());
        let initial_public_keys = genesis_block.validators().unwrap().voting_keys_g2();
        let final_block_number = block.block_number();
        let final_header_hash = <[u8; 32]>::from(block.hash());
        let final_public_keys = block.validators().unwrap().voting_keys_g2();

        // Verify the zk proof.
        let verify_result = NanoZKP::verify(
            genesis_block.block_number(),
            initial_header_hash,
            initial_public_keys,
            final_block_number,
            final_header_hash,
            final_public_keys,
            proof,
            &PathBuf::new(), // use the current directory
        );

        if verify_result.is_err() || !verify_result.unwrap() {
            return Err(PushError::InvalidZKP);
        }

        // At this point we know that the block is correct. We just have to push it.

        // Create the chain info for the new block.
        let block_hash = block.hash();
        let chain_info = ChainInfo::new(block, true);

        read_txn.close();
        let mut txn = this.write_transaction();

        this.state.accounts.reinitialize_as_incomplete(&mut txn);

        // Since it's a macro block, we have to clear the ChainStore. If we are syncing for the first
        // time, this should be empty. But we clear it just in case it's not our first time.
        // Prune the History Store, full nodes will only keep just one epoch of history
        this.history_store.clear(&mut txn);
        // Prune the Chain Store.
        this.chain_store.clear(&mut txn);

        this.chain_store
            .put_chain_info(&mut txn, &block_hash, &chain_info, true);
        this.chain_store.set_head(&mut txn, &block_hash);

        txn.commit();

        // Upgrade the lock as late as possible.
        let mut this = RwLockUpgradableReadGuard::upgrade_untimed(this);

        if let Block::Macro(ref macro_block) = chain_info.head {
            this.state.macro_info = chain_info.clone();
            this.state.macro_head_hash = block_hash.clone();

            this.state.election_head = macro_block.clone();
            this.state.election_head_hash = block_hash.clone();

            // PITODO: Fix this
            this.state.previous_slots = None;

            let new_slots = macro_block.get_validators().unwrap();
            this.state.current_slots.replace(new_slots);

            this.state.can_verify_history = true;
        }

        this.state.main_chain = chain_info;
        this.state.head_hash = block_hash.clone();

        // Downgrade the lock again as the notify listeners might want to acquire read access themselves.
        let this = RwLockWriteGuard::downgrade_to_upgradable(this);

        let num_transactions = this.state.main_chain.head.num_transactions();
        #[cfg(feature = "metrics")]
        this.metrics.note_extend(num_transactions);
        debug!(
            block = %this.state.main_chain.head,
            num_transactions,
            kind = "push_zkp",
            "Accepted block",
        );

        // We shouldn't log errors if there are no listeners.
        _ = this
            .notifier
            .send(BlockchainEvent::EpochFinalized(block_hash));

        // We don't have any block logs, so we do not notify the block log stream.

        Ok(PushResult::Extended)
    }

    /// Pushes an election block backwards into the chain.
    /// This is used to update the `previous_slots` if needed.
    pub fn update_previous_slots(
        this: RwLockUpgradableReadGuard<Self>,
        block: Block,
    ) -> Result<PushResult, PushError> {
        // Must be a macro block.
        assert!(block.is_election());

        // Checks if the body exists.
        block
            .body()
            .ok_or(PushError::InvalidBlock(BlockError::MissingBody))?;

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
        block.verify(false)?;

        // Get epoch number.
        let epoch = Policy::epoch_at(block.block_number());

        // Check if we have this block's successor.
        let prev_block = this
            .chain_store
            .get_block_at(epoch + Policy::blocks_per_epoch(), false, Some(&read_txn))
            .or(Err(PushError::InvalidSuccessor))?;

        // Verify that the block is indeed the predecessor.
        if &block.hash() != prev_block.parent_election_hash().unwrap() {
            return Err(PushError::InvalidPredecessor);
        }

        read_txn.close();

        // Upgrade the blockchain lock
        let mut this = RwLockUpgradableReadGuard::upgrade_untimed(this);

        // Store the election block header.
        if let Block::Macro(ref macro_block) = block {
            this.state.previous_slots = macro_block.get_validators();
        }

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

        // Checks if the body exists.
        block
            .body()
            .ok_or(PushError::InvalidBlock(BlockError::MissingBody))?;

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
        block.verify(false)?;

        // Verify that the block is a valid macro successor to our current macro head.
        block.verify_macro_successor(this.state.macro_info.head.unwrap_macro_ref())?;

        // Verify that the block is valid for the current validators.
        block.verify_validators(&this.current_validators().unwrap())?;

        // At this point we know that the block is correct. We just have to push it.

        // Create the chain info for the new block.
        let block_hash = block.hash();
        let block_number = block.block_number();
        let chain_info = ChainInfo::new(block, true);

        read_txn.close();
        let mut txn = this.write_transaction();

        this.state.accounts.reinitialize_as_incomplete(&mut txn);

        let is_election_block = Policy::is_election_block_at(block_number);

        this.chain_store
            .put_chain_info(&mut txn, &block_hash, &chain_info, true);
        this.chain_store.set_head(&mut txn, &block_hash);

        // Since it's a macro block, we have to clear the ChainStore.
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
        let mut this = RwLockUpgradableReadGuard::upgrade_untimed(this);

        if let Block::Macro(ref macro_block) = chain_info.head {
            this.state.macro_info = chain_info.clone();
            this.state.macro_head_hash = block_hash.clone();
            this.state.can_verify_history = is_election_block;

            if is_election_block {
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

        let num_transactions = this.state.main_chain.head.num_transactions();
        #[cfg(feature = "metrics")]
        this.metrics.note_extend(num_transactions);
        debug!(
            block = %this.state.main_chain.head,
            num_transactions,
            kind = "zkp_extend",
            "Accepted block",
        );

        // We shouldn't log errors if there are no listeners.
        if is_election_block {
            _ = this
                .notifier
                .send(BlockchainEvent::EpochFinalized(block_hash));
        } else {
            _ = this.notifier.send(BlockchainEvent::Finalized(block_hash));
        }

        // We don't have any block logs, so we do not notify the block log stream.

        Ok(PushResult::Extended)
    }
}
