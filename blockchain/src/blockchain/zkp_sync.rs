use std::{cmp, mem};

use nimiq_block::Block;
use nimiq_blockchain_interface::{
    AbstractBlockchain, BlockchainEvent, ChainInfo, PushError, PushResult,
};
use nimiq_database::traits::{ReadTransaction, WriteTransaction};
use nimiq_primitives::policy::Policy;
use nimiq_zkp::{verify::verify, NanoProof, ZKP_VERIFYING_DATA};
use parking_lot::{RwLockUpgradableReadGuard, RwLockWriteGuard};

use crate::{interface::HistoryInterface, Blockchain};

/// Implements methods to sync a full node via ZKP.
impl Blockchain {
    /// Syncs using a zero-knowledge proof. It receives an election block and a proof that there is
    /// a valid chain between the genesis block and that block.
    /// This brings the node from the genesis block all the way to the most recent election block.
    /// It is the default way to sync for a full node.
    ///
    /// When we get a ZKP from the ZKP component, it is already verified.
    /// We can then set the `trusted_proof` flag to avoid the additional verification.
    pub fn push_zkp(
        this: RwLockUpgradableReadGuard<Self>,
        block: Block,
        proof: NanoProof,
        trusted_proof: bool,
    ) -> Result<PushResult, PushError> {
        // Must be an election block.
        assert!(block.is_election());

        let block_hash_blake2b = block.hash();

        let read_txn = this.read_transaction();

        // Check if we already know this block.
        if this
            .chain_store
            .get_chain_info(&block_hash_blake2b, false, Some(&read_txn))
            .is_ok()
        {
            return Ok(PushResult::Known);
        }

        if block.block_number() <= this.state.macro_info.head.block_number() {
            return Ok(PushResult::Ignored);
        }

        // Perform block intrinsic checks.
        block.verify(this.network_id)?;

        // Prepare the inputs to verify the proof.
        let genesis_block = this
            .chain_store
            .get_block_at(Policy::genesis_block_number(), true, Some(&read_txn))
            .unwrap();
        let genesis_macro_block = genesis_block.unwrap_macro_ref();
        let genesis_hash_blake2s = genesis_macro_block.hash_blake2s();
        let genesis_hash_blake2b = genesis_macro_block.hash();

        // Verify the zk proof.
        if !trusted_proof {
            let verify_result = verify(
                genesis_hash_blake2s,
                block.unwrap_macro_ref().hash_blake2s(),
                proof,
                &ZKP_VERIFYING_DATA,
            );

            if verify_result.is_err() || !verify_result.unwrap() {
                return Err(PushError::InvalidZKP);
            }
        }

        // At this point we know that the block is correct. We just have to push it.

        // Create the chain info for the new block.
        let chain_info = ChainInfo::new(block, true);

        read_txn.close();
        let mut txn = this.write_transaction();

        this.state
            .accounts
            .reinitialize_as_incomplete(&mut (&mut txn).into());

        // Since it's a macro block, we have to clear the ChainStore. If we are syncing for the first
        // time, this should be empty. But we clear it just in case it's not our first time.
        // Prune the History Store, full nodes will only keep just one epoch of history
        this.history_store.clear(&mut txn);
        // Prune the Chain Store.
        this.chain_store.clear(&mut txn);

        // Restore genesis block.
        let genesis_info = ChainInfo::new(genesis_block, true);
        this.chain_store
            .put_chain_info(&mut txn, &genesis_hash_blake2b, &genesis_info, true);

        this.chain_store
            .put_chain_info(&mut txn, &block_hash_blake2b, &chain_info, true);
        this.chain_store.set_head(&mut txn, &block_hash_blake2b);

        txn.commit();

        // Upgrade the lock as late as possible.
        let mut this = RwLockUpgradableReadGuard::upgrade(this);

        if let Block::Macro(ref macro_block) = chain_info.head {
            this.state.macro_info = chain_info.clone();
            this.state.macro_head_hash = block_hash_blake2b.clone();

            this.state.election_head = macro_block.clone();
            let old_election_head_hash = mem::replace(
                &mut this.state.election_head_hash,
                block_hash_blake2b.clone(),
            );

            // If we coincidentally know the previous election block, we remember its slots.
            if old_election_head_hash == macro_block.header.parent_election_hash {
                let old_slots = this.state.current_slots.take().unwrap();
                this.state.previous_slots.replace(old_slots);
            } else {
                this.state.previous_slots = None;
            }

            let new_slots = macro_block.get_validators().unwrap();
            this.state.current_slots.replace(new_slots);
        }

        this.state.main_chain = chain_info;
        this.state.head_hash = block_hash_blake2b.clone();

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
        this.notifier
            .send(BlockchainEvent::Extended(block_hash_blake2b.clone()))
            .ok();

        this.notifier
            .send(BlockchainEvent::EpochFinalized(block_hash_blake2b))
            .ok();

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
        block.verify(this.network_id)?;

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
            kind = "zkp_update_slots",
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
        block.verify(this.network_id)?;

        // Verify that the block is a valid macro successor to our current macro head.
        block.verify_macro_successor(this.state.macro_info.head.unwrap_macro_ref())?;

        // Verify that the block is valid for the current validators.
        block.verify_validators(this.current_validators().unwrap())?;

        // At this point we know that the block is correct. We just have to push it.

        // Create the chain info for the new block.
        let block_hash = block.hash();
        let block_number = block.block_number();
        let chain_info = ChainInfo::new(block, true);

        read_txn.close();
        let mut txn = this.write_transaction();

        this.state
            .accounts
            .reinitialize_as_incomplete(&mut (&mut txn).into());

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
            kind = "push_macro",
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
        } else {
            this.notifier
                .send(BlockchainEvent::Finalized(block_hash))
                .ok();
        }

        // We don't have any block logs, so we do not notify the block log stream.

        Ok(PushResult::Extended)
    }
}
