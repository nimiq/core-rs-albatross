use std::ops::Deref;

use nimiq_block::{Block, BlockError, ForkProof, MacroHeader, MicroBlock};
use nimiq_blockchain_interface::{
    AbstractBlockchain, BlockchainEvent, ChainInfo, ChainOrdering, ForkEvent, PushError, PushResult,
};
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_keys::Address;
use nimiq_primitives::policy::Policy;
use parking_lot::RwLockUpgradableReadGuard;

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
        // Ignore all blocks that precede (or are at the same height) as the most recent accepted
        // macro block.
        let last_macro_block = Policy::last_macro_block(this.block_number());
        if block.block_number() <= last_macro_block {
            log::debug!(
                block_no = block.block_number(),
                reason = "we have already finalized a later macro block",
                last_macro_block_no = last_macro_block,
                "Ignoring block",
            );
            return Ok(PushResult::Ignored);
        }

        // Check if we already know this block.
        if this.get_chain_info(&block.hash(), false).is_ok() {
            return Ok(PushResult::Known);
        }

        // Check if we have this block's parent.
        let prev_info = this
            .get_chain_info(block.parent_hash(), false)
            .map_err(|error| {
                log::warn!(
                    %error,
                    %block,
                    reason = "parent block is unknown",
                    parent_block_hash = %block.parent_hash(),
                    "Rejecting block",
                );
                PushError::Orphan
            })?;

        // Calculate chain ordering.
        let chain_order = ChainOrdering::order_chains(
            this.deref(),
            &block,
            &prev_info,
            |hash| this.get_chain_info(hash, false),
            |height| this.get_block_at(height, false),
        );

        // We expect full blocks (with body) for macro blocks and no body for micro blocks.
        if block.is_macro() {
            block
                .body()
                .ok_or(PushError::InvalidBlock(BlockError::MissingBody))?;
        } else {
            assert!(
                block.body().is_none(),
                "Light blockchain expects micro blocks without body"
            )
        }

        // Perform block intrinsic checks.
        block.verify()?;

        // Verify that the block is a valid immediate successor to its predecessor.
        let predecessor = &prev_info.head;
        block.verify_immediate_successor(predecessor)?;

        // Verify that the block is a valid macro successor to our current macro head.
        if block.is_macro() {
            block.verify_macro_successor(&this.macro_head)?;
        }

        // Get the slot owner of the block. Note that the predecessor must be taken into account
        // as the block is not guaranteed to be on the current main chain.
        let offset = block
            .vrf_offset()
            .ok_or(PushError::InvalidBlock(BlockError::MissingJustification))?;
        let proposer = this
            .get_proposer(
                block.block_number(),
                offset,
                predecessor.seed().entropy(),
            )
            .map_err(|error| {
                log::warn!(%error, %block, reason = "Failed to determine block proposer", "Rejecting block");
                PushError::Orphan
            })?
            .validator;

        // Verify that the block is valid for the given proposer.
        block.verify_proposer(&proposer.signing_key, predecessor.seed())?;

        // Verify that the block is valid for the current validators.
        block.verify_validators(&this.current_validators().unwrap())?;

        // Detect forks in non-skip micro blocks.
        if block.is_micro() && !block.is_skip() {
            let slot = this
                .get_proposer(
                    block.block_number(),
                    block.block_number(),
                    prev_info.head.seed().entropy(),
                )
                .expect("Couldn't find slot owner");
            this.detect_forks(block.unwrap_micro_ref(), &slot.validator.address);
        }

        // Create the chain info for the new block.
        let chain_info = ChainInfo::from_block(block, &prev_info, None);

        // More chain ordering.
        let result = match chain_order {
            ChainOrdering::Extend => {
                return LightBlockchain::extend(this, chain_info, prev_info);
            }
            ChainOrdering::Superior => {
                return LightBlockchain::rebranch(this, chain_info);
            }
            ChainOrdering::Inferior => {
                log::debug!(block = %chain_info.head, "Storing block - on inferior chain");
                PushResult::Ignored
            }
            ChainOrdering::Unknown => {
                log::debug!(block = %chain_info.head, "Storing block - on fork");
                PushResult::Forked
            }
        };
        let mut this = RwLockUpgradableReadGuard::upgrade(this);
        // Otherwise, we are creating/extending a fork. Store ChainInfo.
        this.chain_store.put_chain_info(chain_info);

        Ok(result)
    }

    /// Extends the current main chain.
    fn extend(
        this: RwLockUpgradableReadGuard<Self>,
        mut chain_info: ChainInfo,
        mut prev_info: ChainInfo,
    ) -> Result<PushResult, PushError> {
        // Upgrade the blockchain lock
        let mut this = RwLockUpgradableReadGuard::upgrade(this);

        let is_election_block = Policy::is_election_block_at(chain_info.head.block_number());
        let is_macro_block = Policy::is_macro_block_at(chain_info.head.block_number());
        let block_hash = chain_info.head.hash();

        // Update chain infos.
        chain_info.on_main_chain = true;
        prev_info.main_chain_successor = Some(block_hash.clone());

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

        log::debug!(
            block = %this.head,
            kind = "extend",
            "Accepted block",
        );

        // We shouldn't log errors if there are no listeners.
        if is_election_block {
            _ = this
                .notifier
                .send(BlockchainEvent::EpochFinalized(block_hash));
        } else if is_macro_block {
            _ = this.notifier.send(BlockchainEvent::Finalized(block_hash));
        } else {
            _ = this.notifier.send(BlockchainEvent::Extended(block_hash));
        }

        Ok(PushResult::Extended)
    }

    /// Rebranches the current main chain.
    fn rebranch(
        this: RwLockUpgradableReadGuard<Self>,
        chain_info: ChainInfo,
    ) -> Result<PushResult, PushError> {
        // Upgrade the blockchain lock
        let mut this = RwLockUpgradableReadGuard::upgrade(this);

        let target_block = chain_info.head.header();
        log::debug!(block = %target_block, "Rebranching");

        // Find the common ancestor between our current main chain and the fork chain.
        // Walk up the fork chain until we find a block that is part of the main chain.
        // Store the chain along the way.

        let mut fork_chain: Vec<(Blake2bHash, ChainInfo)> = vec![];
        let mut current: (Blake2bHash, ChainInfo) = (chain_info.head.hash(), chain_info);

        while !current.1.on_main_chain {
            let prev_hash = current.1.head.parent_hash().clone();

            let prev_info = this
                .get_chain_info(&prev_hash, false)
                .expect("Corrupted store: Failed to find fork predecessor while rebranching");

            fork_chain.push(current);

            current = (prev_hash, prev_info);
        }

        log::debug!(
            block = %target_block,
            common_ancestor = %current.1.head,
            no_blocks_up = fork_chain.len(),
            "Found common ancestor",
        );

        // Revert to the common ancestor.
        let mut revert_chain: Vec<(Blake2bHash, ChainInfo)> = vec![];
        let mut ancestor = current;

        // Check if ancestor is in current batch.
        if ancestor.1.head.block_number() < this.macro_head.block_number() {
            log::warn!(
                block = %target_block,
                reason = "ancestor block already finalized",
                ancestor_block = %ancestor.1.head,
                "Rejecting block",
            );
            return Err(PushError::InvalidFork);
        }

        current = (
            this.head_hash(),
            this.get_chain_info(&this.head_hash(), false)
                .expect("Couldn't find the head chain info!"),
        );

        while current.0 != ancestor.0 {
            let block = current.1.head.clone();
            if block.is_macro() {
                panic!("Trying to rebranch across macro block");
            }

            let prev_hash = block.parent_hash().clone();

            let prev_info = this
                .get_chain_info(&prev_hash, false)
                .expect("Corrupted store: Failed to find fork predecessor while rebranching");

            revert_chain.push(current);

            current = (prev_hash, prev_info);
        }

        // Unset on_main_chain flag / main_chain_successor on the current main chain up to (excluding) the common ancestor.
        for reverted_block in revert_chain.iter_mut() {
            reverted_block.1.on_main_chain = false;
            reverted_block.1.main_chain_successor = None;

            this.chain_store.put_chain_info(reverted_block.1.clone());
        }

        // Update the main_chain_successor of the common ancestor block.
        ancestor.1.main_chain_successor = Some(fork_chain.last().unwrap().0.clone());
        this.chain_store.put_chain_info(ancestor.1);

        // Set on_main_chain flag / main_chain_successor on the fork.
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
            this.chain_store.put_chain_info(fork_block.1.clone());
        }

        // Update head.
        let new_head_info = &fork_chain[0].1;

        this.head = new_head_info.head.clone();

        let mut reverted_blocks = Vec::with_capacity(revert_chain.len());
        for (hash, chain_info) in revert_chain.into_iter().rev() {
            log::debug!(
                block = %chain_info.head,
                num_transactions = chain_info.head.num_transactions(),
                "Reverted block",
            );
            reverted_blocks.push((hash, chain_info.head));
        }

        let mut adopted_blocks = Vec::with_capacity(fork_chain.len());
        for (hash, chain_info) in fork_chain.into_iter().rev() {
            log::debug!(
                block = %chain_info.head,
                num_transactions = chain_info.head.num_transactions(),
                kind = "rebranch",
                "Accepted block",
            );
            adopted_blocks.push((hash, chain_info.head));
        }

        log::debug!(
            block = %this.head,
            num_reverted_blocks = reverted_blocks.len(),
            num_adopted_blocks = adopted_blocks.len(),
            "Rebranched",
        );

        // We do not log errors if there are no listeners
        _ = this
            .notifier
            .send(BlockchainEvent::Rebranched(reverted_blocks, adopted_blocks));

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
        let mut this = RwLockUpgradableReadGuard::upgrade(this);

        // Store the election block header.
        this.chain_store.put_election(header);

        Ok(PushResult::Extended)
    }

    fn detect_forks(&self, block: &MicroBlock, validator_address: &Address) {
        assert!(!block.is_skip_block());

        // Check if there are two blocks in the same slot and with the same height. Since we already
        // verified the validator for the current slot, this is enough to check for fork proofs.
        // Note: We don't verify the justifications for the other blocks here, since they already
        // had to be verified in order to be added to the blockchain.
        let mut micro_blocks: Vec<Block> =
            self.chain_store.get_blocks_at(block.header.block_number);

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
