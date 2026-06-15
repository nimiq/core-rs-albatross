use nimiq_block::Block;
use nimiq_blockchain_interface::{
    AbstractBlockchain, BlockchainEvent, ChainInfo, PushError, PushResult,
};
use nimiq_primitives::policy::Policy;
use parking_lot::RwLockUpgradableReadGuard;

use crate::blockchain::LightBlockchain;

/// Implements methods to sync a light node.
impl LightBlockchain {
    /// Pushes a macro block into the blockchain. This is used when we have already synced to the
    /// most recent election block and now need to push a checkpoint block.
    /// But this function is general enough to allow pushing any macro block (checkpoint or election)
    /// at any state of the node (synced, partially synced, not synced).
    pub fn push_macro(
        this: RwLockUpgradableReadGuard<Self>,
        mut block: Block,
    ) -> Result<PushResult, PushError> {
        // Must be a macro block.
        assert!(block.is_macro());

        let block_hash = block.hash_cached();

        // Check if we already know this block.
        if this.chain_store.get_chain_info(&block_hash, false).is_ok() {
            return Ok(PushResult::Known);
        }

        if block.block_number() <= this.macro_head.block_number() {
            return Ok(PushResult::Ignored);
        }

        // We expect blocks without body here. Defensively strip the block body as opposed to
        // rejecting the block if the body is present as we can still push it just fine.
        match block {
            Block::Macro(ref mut block) => block.body = None,
            Block::Micro(ref mut block) => block.body = None,
        }

        // Perform block intrinsic checks.
        let max_timestamp = this.now().saturating_add(Policy::TIMESTAMP_MAX_DRIFT);
        block.verify(this.network_id, max_timestamp)?;

        // Verify that the block is a valid macro successor to our current macro head.
        block.verify_macro_successor(&this.macro_head)?;

        // Verify that the block is valid for the current validators.
        block.verify_validators(this.current_validators().unwrap())?;

        // At this point we know that the block is correct. We just have to push it.

        // Upgrade the blockchain lock
        let mut this = RwLockUpgradableReadGuard::upgrade(this);

        // Create the chain info for the new block.
        let chain_info = ChainInfo::new(block.clone(), true);

        // Remove old blocks from the ChainStore.
        this.chain_store
            .clear_old_blocks(chain_info.head.block_number());

        // Store the block chain info.
        this.chain_store.put_chain_info(chain_info);

        // Update the blockchain.
        this.head = block.clone();

        this.macro_head = block.clone().unwrap_macro();

        this.notifier
            .send(BlockchainEvent::Extended(block_hash.clone()))
            .ok();

        // If it's an election block, you have more steps.
        if block.is_election() {
            this.election_head = block.unwrap_macro_ref().clone();

            this.current_validators = block.validators();

            // Store the election block header.
            this.chain_store.put_election(block.unwrap_macro().header);

            // We shouldn't log errors if there are no listeners.
            this.notifier
                .send(BlockchainEvent::EpochFinalized(block_hash))
                .ok();
        } else {
            // We shouldn't log errors if there are no listeners.
            this.notifier
                .send(BlockchainEvent::Finalized(block_hash))
                .ok();
        }

        Ok(PushResult::Extended)
    }

    /// Pushes an election block for the pico sync, bypassing many checks
    pub fn push_pico_election(
        this: RwLockUpgradableReadGuard<Self>,
        mut block: Block,
    ) -> Result<PushResult, PushError> {
        // Must be a macro block.
        assert!(block.is_macro());
        assert!(Policy::is_election_block_at(block.block_number()));

        let block_hash = block.hash_cached();

        // Check if we already know this block.
        if this.chain_store.get_chain_info(&block_hash, false).is_ok() {
            return Ok(PushResult::Known);
        }

        if block.block_number() <= this.macro_head.block_number() {
            return Ok(PushResult::Ignored);
        }

        // We expect blocks without body here. Defensively strip the block body as opposed to
        // rejecting the block if the body is present as we can still push it just fine.
        match block {
            Block::Macro(ref mut block) => block.body = None,
            Block::Micro(ref mut block) => block.body = None,
        }

        // Perform block intrinsic checks.
        let max_timestamp = this.now().saturating_add(Policy::TIMESTAMP_MAX_DRIFT);
        block.verify(this.network_id, max_timestamp)?;

        // Upgrade the blockchain lock
        let mut this = RwLockUpgradableReadGuard::upgrade(this);

        // Create the chain info for the new block.
        let chain_info = ChainInfo::new(block.clone(), true);

        // Remove old blocks from the ChainStore.
        this.chain_store
            .clear_old_blocks(chain_info.head.block_number());

        // Store the block chain info.
        this.chain_store.put_chain_info(chain_info);

        // Update the blockchain.
        this.head = block.clone();

        this.macro_head = block.clone().unwrap_macro();

        this.notifier
            .send(BlockchainEvent::Extended(block_hash.clone()))
            .ok();

        this.election_head = block.unwrap_macro_ref().clone();

        this.current_validators = block.validators();

        // Store the election block header.
        this.chain_store.put_election(block.unwrap_macro().header);

        // We shouldn't log errors if there are no listeners.
        this.notifier
            .send(BlockchainEvent::EpochFinalized(block_hash))
            .ok();

        Ok(PushResult::Extended)
    }
}
