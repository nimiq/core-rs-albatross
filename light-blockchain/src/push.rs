use nimiq_block::{Block, BlockError, BlockType, MacroHeader};
use nimiq_blockchain::{AbstractBlockchain, ChainInfo, ChainOrdering, PushError, PushResult};
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_primitives::policy::Policy;
use parking_lot::RwLockUpgradableReadGuard;
use std::ops::Deref;

use crate::blockchain::LightBlockchain;

/// Implements methods to push blocks into the chain. This is used when the node has already synced
/// and is just receiving newly produced blocks. It is also used for the final phase of syncing,
/// when the node is just receiving micro blocks.
impl LightBlockchain {
    /// Pushes a block into the chain.
    pub fn push(
        this: RwLockUpgradableReadGuard<Self>,
        block: Block,
    ) -> Result<PushResult, PushError> {
        // Check if we already know this block.
        if this.get_chain_info(&block.hash(), false, None).is_some() {
            return Ok(PushResult::Known);
        }

        // Check if we have this block's parent.
        let prev_info = this
            .get_chain_info(block.parent_hash(), false, None)
            .ok_or(PushError::Orphan)?;

        // Calculate chain ordering.
        let chain_order = ChainOrdering::order_chains(this.deref(), &block, &prev_info, None);

        // If it is an inferior chain, we ignore it as it cannot become better at any point in time.
        if chain_order == ChainOrdering::Inferior {
            return Ok(PushResult::Ignored);
        }

        // Get the intended slot owner.
        let offset = if let Block::Macro(macro_block) = &block {
            if let Some(proof) = &macro_block.justification {
                proof.round
            } else {
                return Err(PushError::InvalidBlock(BlockError::MissingJustification));
            }
        } else {
            block.block_number()
        };
        let (slot_owner, _) = this
            .get_slot_owner_at(block.block_number(), offset, None)
            .expect("Failed to find slot owner!");

        // Perform block intrinsic checks.
        block.verify(false)?;

        // Verify that the block is a valid immediate successor to its predecessor.
        let predecessor = &prev_info.head;
        block.verify_immediate_successor(predecessor)?;

        // Verify that the block is a valid macro successor to our current macro head.
        if block.is_macro() {
            block.verify_macro_successor(&this.macro_head)?;
        }

        // Verify that the block is valid for the given proposer.
        block.verify_proposer(&slot_owner.signing_key, predecessor.seed())?;

        // Verify that the block is valid for the current validators.
        block.verify_validators(&this.current_validators().unwrap())?;

        // Create the chaininfo for the new block.
        let chain_info = ChainInfo::from_block(block, &prev_info);

        // More chain ordering.
        match chain_order {
            ChainOrdering::Extend => {
                return LightBlockchain::extend(this, chain_info, prev_info);
            }
            ChainOrdering::Superior => {
                return LightBlockchain::rebranch(this, chain_info, prev_info);
            }
            ChainOrdering::Inferior => unreachable!(),
            ChainOrdering::Unknown => {}
        }
        let mut this = RwLockUpgradableReadGuard::upgrade_untimed(this);
        // Otherwise, we are creating/extending a fork. Store ChainInfo.
        this.chain_store.put_chain_info(chain_info);

        Ok(PushResult::Forked)
    }

    /// Extends the current main chain.
    fn extend(
        this: RwLockUpgradableReadGuard<Self>,
        mut chain_info: ChainInfo,
        mut prev_info: ChainInfo,
    ) -> Result<PushResult, PushError> {
        // Upgrade the blockchain lock
        let mut this = RwLockUpgradableReadGuard::upgrade_untimed(this);

        // Update chain infos.
        chain_info.on_main_chain = true;
        prev_info.main_chain_successor = Some(chain_info.head.hash());

        // If it's a macro block then we need to clear the ChainStore (since we only want to keep
        // the current batch in memory). Otherwise, we need to update the previous ChainInfo.
        if chain_info.head.is_macro() {
            this.chain_store.clear();
        } else {
            this.chain_store.put_chain_info(prev_info);
        }

        // Update the head of the blockchain.
        this.head = chain_info.head.clone();

        // If the block is a macro block then we also need to update the macro head.
        if let Block::Macro(ref macro_block) = chain_info.head {
            this.macro_head = macro_block.clone();

            // If the block is also an election block, then we have more fields to update.
            if macro_block.is_election_block() {
                this.election_head = macro_block.clone();

                this.current_validators = macro_block.get_validators();

                // Store the election block header.
                this.chain_store.put_election(macro_block.header.clone());
            }
        }

        // Store the current chain info.
        this.chain_store.put_chain_info(chain_info);

        Ok(PushResult::Extended)
    }

    /// Rebranches the current main chain.
    fn rebranch(
        this: RwLockUpgradableReadGuard<Self>,
        mut chain_info: ChainInfo,
        mut prev_info: ChainInfo,
    ) -> Result<PushResult, PushError> {
        // You can't rebranch a macro block.
        assert!(chain_info.head.is_micro());

        // Upgrade the blockchain lock
        let mut this = RwLockUpgradableReadGuard::upgrade_untimed(this);

        // Update chain infos.
        chain_info.on_main_chain = true;
        prev_info.main_chain_successor = Some(chain_info.head.hash());

        // Find the common ancestor between our current main chain and the fork chain.
        // Walk up the fork chain until we find a block that is part of the main chain.
        // Update the fork chain along the way.
        let mut current = prev_info;

        while !current.on_main_chain {
            // A fork can't contain a macro block. We already received that macro block, thus it must be on our
            // main chain.
            assert_eq!(current.head.ty(), BlockType::Micro);

            // Get previous chain info.
            let prev_info = this
                .get_chain_info(current.head.parent_hash(), false, None)
                .expect("Corrupted store: Failed to find fork predecessor while rebranching");

            // Update the chain info.
            current.on_main_chain = true;

            // Store the chain info.
            this.chain_store.put_chain_info(current);

            current = prev_info;
        }

        // Remember the ancestor block.
        let ancestor = current;

        // Go back from the head of the forked chain until the ancestor, updating it along the way.
        current = this
            .get_chain_info(&this.head_hash(), false, None)
            .expect("Couldn't find the head chain info!");

        while current != ancestor {
            // Get previous chain info.
            let prev_info = this
                .get_chain_info(current.head.parent_hash(), false, None)
                .expect("Corrupted store: Failed to find fork predecessor while rebranching");

            // Update the chain info.
            current.on_main_chain = false;

            // Store the chain info.
            this.chain_store.put_chain_info(current);

            current = prev_info;
        }

        // Update the head of the blockchain.
        this.head = chain_info.head;

        Ok(PushResult::Rebranched)
    }

    /// Pushes an election block backwards into the chain. This pushes the election block immediately
    /// before the oldest election block that we have. It is useful in case we need to receive a proof
    /// for a transaction in a past epoch, in that case the simplest course of action is to "walk"
    /// backwards from our current election block until we get to the desired epoch.
    pub fn push_election_backwards(
        this: RwLockUpgradableReadGuard<Self>,
        header: MacroHeader,
    ) -> Result<PushResult, PushError> {
        // Get epoch number.
        let epoch = Policy::epoch_at(header.block_number);

        // Check if we already know this block.
        if this.chain_store.get_election(epoch).is_some() {
            return Ok(PushResult::Known);
        }

        // Check if we have this block's successor.
        let prev_block = this
            .chain_store
            .get_election(epoch + Policy::blocks_per_epoch())
            .ok_or(PushError::InvalidSuccessor)?;

        // Verify that the block is indeed the predecessor.
        if header.hash::<Blake2bHash>() != prev_block.parent_election_hash {
            return Err(PushError::InvalidPredecessor);
        }

        // Upgrade the blockchain lock
        let mut this = RwLockUpgradableReadGuard::upgrade_untimed(this);

        // Store the election block header.
        this.chain_store.put_election(header);

        Ok(PushResult::Extended)
    }
}
