use std::ops::Deref;

use parking_lot::{RwLockUpgradableReadGuard, RwLockWriteGuard};

use nimiq_block::{Block, ForkProof};
use nimiq_database::WriteTransaction;
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_primitives::policy;

use crate::blockchain_state::BlockchainState;
use crate::chain_info::ChainInfo;
use crate::chain_store::MAX_EPOCHS_STORED;
use crate::{
    AbstractBlockchain, Blockchain, BlockchainEvent, ChainOrdering, ForkEvent, PushError,
    PushResult,
};

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
    ) -> Result<PushResult, PushError> {
        // Ignore all blocks that precede (or are at the same height) as the most recent accepted
        // macro block.
        if block.block_number() <= policy::last_macro_block(this.block_number()) {
            info!(
                "Ignoring block (#{}.{}, {})- we already know a later macro block (we're at {})",
                block.block_number(),
                block.view_number(),
                block.hash(),
                this.block_number()
            );
            return Ok(PushResult::Ignored);
        }

        // TODO: We might want to pass this as argument to this method.
        let read_txn = this.read_transaction();

        // Check if we already know this block.
        if this
            .chain_store
            .get_chain_info(&block.hash(), false, Some(&read_txn))
            .is_some()
        {
            return Ok(PushResult::Known);
        }

        // Check if we have this block's parent.
        let prev_info = this
            .chain_store
            .get_chain_info(block.parent_hash(), false, Some(&read_txn))
            .ok_or(PushError::Orphan)?;

        // Get the intended slot owner.
        let (slot_owner, _) = this
            .get_slot_owner_at(block.block_number(), block.view_number(), Some(&read_txn))
            .expect("Couldn't calculate slot owner!");

        // Check the header.
        if let Err(e) = Blockchain::verify_block_header(
            this.deref(),
            &block.header(),
            &slot_owner.signing_key,
            Some(&read_txn),
            !trusted,
        ) {
            warn!("Rejecting block - Bad header");
            return Err(e);
        }

        // Check the justification.
        if let Err(e) = Blockchain::verify_block_justification(
            &*this,
            &block,
            &slot_owner.signing_key,
            Some(&read_txn),
            !trusted,
        ) {
            warn!("Rejecting block - Bad justification");
            return Err(e);
        }

        // Check the body.
        if let Err(e) =
            this.verify_block_body(&block.header(), &block.body(), Some(&read_txn), !trusted)
        {
            warn!("Rejecting block - Bad body");
            return Err(e);
        }

        // Detect forks.
        if let Block::Micro(micro_block) = &block {
            // Check if there are two blocks in the same slot and with the same height. Since we already
            // verified the validator for the current slot, this is enough to check for fork proofs.
            // Note: We don't verify the justifications for the other blocks here, since they had to
            // already be verified in order to be added to the blockchain.
            // Count the micro blocks after the last macro block.
            let mut micro_blocks: Vec<Block> =
                this.chain_store
                    .get_blocks_at(block.block_number(), false, Some(&read_txn));

            // Get the micro header from the block
            let micro_header1 = &micro_block.header;

            // Get the justification for the block. We assume that the
            // validator's signature is valid.
            let justification1 = &micro_block
                .justification
                .as_ref()
                .expect("Missing justification!")
                .signature;

            // Get the view number from the block
            let view_number = block.view_number();

            for micro_block in micro_blocks.drain(..).map(|block| block.unwrap_micro()) {
                // If there's another micro block set to this view number, which also has the same
                // VrfSeed entropy we notify the fork event.
                if view_number == micro_block.header.view_number
                    && &block.seed().entropy() == &micro_block.header.seed.entropy()
                {
                    let micro_header2 = micro_block.header;
                    let justification2 = micro_block
                        .justification
                        .expect("Missing justification!")
                        .signature;

                    let proof = ForkProof {
                        header1: micro_header1.clone(),
                        header2: micro_header2,
                        justification1: justification1.clone(),
                        justification2,
                        prev_vrf_seed: prev_info.head.seed().clone(),
                    };

                    this.fork_notifier.notify(ForkEvent::Detected(proof));
                }
            }
        }

        // Calculate chain ordering.
        let chain_order =
            ChainOrdering::order_chains(this.deref(), &block, &prev_info, Some(&read_txn));

        read_txn.close();

        let chain_info = ChainInfo::from_block(block, &prev_info);

        // Extend, rebranch or just store the block depending on the chain ordering.
        let result = match chain_order {
            ChainOrdering::Extend => {
                return Blockchain::extend(this, chain_info.head.hash(), chain_info, prev_info);
            }
            ChainOrdering::Superior => {
                return Blockchain::rebranch(this, chain_info.head.hash(), chain_info);
            }
            ChainOrdering::Inferior => {
                debug!(
                    "Ignoring inferior block #{}.{} {}",
                    chain_info.head.block_number(),
                    chain_info.head.view_number(),
                    chain_info.head.hash(),
                );
                PushResult::Ignored
            }
            ChainOrdering::Unknown => {
                debug!(
                    "Creating/extending fork with block #{}.{} {}",
                    chain_info.head.block_number(),
                    chain_info.head.view_number(),
                    chain_info.head.hash(),
                );
                PushResult::Forked
            }
        };

        let mut txn = this.write_transaction();
        this.chain_store
            .put_chain_info(&mut txn, &chain_info.head.hash(), &chain_info, true);
        txn.commit();

        Ok(result)
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
        Self::do_push(this, block, false)
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
        Self::do_push(this, block, true)
    }

    /// Extends the current main chain.
    fn extend(
        this: RwLockUpgradableReadGuard<Blockchain>,
        block_hash: Blake2bHash,
        mut chain_info: ChainInfo,
        mut prev_info: ChainInfo,
    ) -> Result<PushResult, PushError> {
        let mut txn = this.write_transaction();

        let block_number = this.block_number() + 1;
        let is_macro_block = policy::is_macro_block_at(block_number);
        let is_election_block = policy::is_election_block_at(block_number);

        if let Err(e) = this.check_and_commit(
            &this.state,
            &chain_info.head,
            prev_info.head.next_view_number(),
            &mut txn,
        ) {
            txn.abort();
            return Err(e);
        }

        chain_info.on_main_chain = true;
        prev_info.main_chain_successor = Some(chain_info.head.hash());

        this.chain_store
            .put_chain_info(&mut txn, &block_hash, &chain_info, true);
        this.chain_store
            .put_chain_info(&mut txn, chain_info.head.parent_hash(), &prev_info, false);
        this.chain_store.set_head(&mut txn, &block_hash);

        if is_election_block {
            this.chain_store.prune_epoch(
                policy::epoch_at(block_number).saturating_sub(MAX_EPOCHS_STORED),
                &mut txn,
            );
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

        // Downgrade the lock again as the nofity listeners might want to acquire read access themselves.
        let this = RwLockWriteGuard::downgrade_to_upgradable(this);

        if is_election_block {
            this.notifier
                .notify(BlockchainEvent::EpochFinalized(block_hash));
        } else if is_macro_block {
            this.notifier.notify(BlockchainEvent::Finalized(block_hash));
        } else {
            this.notifier.notify(BlockchainEvent::Extended(block_hash));
        }

        Ok(PushResult::Extended)
    }

    /// Rebranches the current main chain.
    fn rebranch(
        this: RwLockUpgradableReadGuard<Blockchain>,
        block_hash: Blake2bHash,
        chain_info: ChainInfo,
    ) -> Result<PushResult, PushError> {
        debug!(
            "Rebranching to fork {}, height #{}, view number {}",
            block_hash,
            chain_info.head.block_number(),
            chain_info.head.view_number()
        );

        // Find the common ancestor between our current main chain and the fork chain.
        // Walk up the fork chain until we find a block that is part of the main chain.
        // Store the chain along the way.
        let read_txn = this.read_transaction();

        let mut fork_chain: Vec<(Blake2bHash, ChainInfo)> = vec![];

        let mut current: (Blake2bHash, ChainInfo) = (block_hash, chain_info);

        while !current.1.on_main_chain {
            let prev_hash = current.1.head.parent_hash().clone();

            let prev_info = this
                .chain_store
                .get_chain_info(&prev_hash, true, Some(&read_txn))
                .expect("Corrupted store: Failed to find fork predecessor while rebranching");

            fork_chain.push(current);

            current = (prev_hash, prev_info);
        }
        read_txn.close();

        debug!(
            "Found common ancestor {} at height #{}, {} blocks up",
            current.0,
            current.1.head.block_number(),
            fork_chain.len()
        );

        // Revert AccountsTree & TransactionCache to the common ancestor state.
        let mut revert_chain: Vec<(Blake2bHash, ChainInfo)> = vec![];
        let mut ancestor = current;

        // Check if ancestor is in current batch.
        if ancestor.1.head.block_number() < this.state.macro_info.head.block_number() {
            info!("Ancestor is in finalized epoch");
            return Err(PushError::InvalidFork);
        }

        let mut write_txn = this.write_transaction();

        current = (this.state.head_hash.clone(), this.state.main_chain.clone());

        while current.0 != ancestor.0 {
            match current.1.head {
                Block::Macro(_) => {
                    // Macro blocks are final, we can't revert across them. This should be checked
                    // before we start reverting.
                    panic!("Trying to rebranch across macro block");
                }
                Block::Micro(ref micro_block) => {
                    let prev_hash = micro_block.header.parent_hash.clone();

                    let prev_info = this
                        .chain_store
                        .get_chain_info(&prev_hash, true, Some(&write_txn))
                        .expect("Corrupted store: Failed to find main chain predecessor while rebranching");

                    this.revert_accounts(
                        &this.state.accounts,
                        &mut write_txn,
                        micro_block,
                        prev_info.head.next_view_number(),
                    )?;

                    assert_eq!(
                        prev_info.head.state_root(),
                        &this.state.accounts.get_root(Some(&write_txn)),
                        "Failed to revert main chain while rebranching - inconsistent state"
                    );

                    revert_chain.push(current);

                    current = (prev_hash, prev_info);
                }
            }
        }

        // Push each fork block.
        let mut prev_view_number = ancestor.1.head.next_view_number();

        let mut fork_iter = fork_chain.iter().rev();

        while let Some(fork_block) = fork_iter.next() {
            if let Err(e) = this.check_and_commit(
                &this.state,
                &fork_block.1.head,
                prev_view_number,
                &mut write_txn,
            ) {
                warn!("Failed to apply fork block while rebranching - {:?}", e);
                write_txn.abort();

                // Delete invalid fork blocks from store.
                let mut write_txn = this.write_transaction();
                for block in vec![fork_block].into_iter().chain(fork_iter) {
                    this.chain_store.remove_chain_info(
                        &mut write_txn,
                        &block.0,
                        fork_block.1.head.block_number(),
                    )
                }
                write_txn.commit();

                return Err(PushError::InvalidFork);
            }

            prev_view_number = fork_block.1.head.next_view_number();
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

            if policy::is_election_block_at(new_head_info.head.block_number()) {
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

        let mut reverted_blocks = Vec::with_capacity(revert_chain.len());
        for (hash, chain_info) in revert_chain.into_iter().rev() {
            reverted_blocks.push((hash, chain_info.head));
        }

        let mut adopted_blocks = Vec::with_capacity(fork_chain.len());
        for (hash, chain_info) in fork_chain.into_iter().rev() {
            adopted_blocks.push((hash, chain_info.head));
        }

        let event = BlockchainEvent::Rebranched(reverted_blocks, adopted_blocks);
        this.notifier.notify(event);

        Ok(PushResult::Rebranched)
    }

    fn check_and_commit(
        &self,
        state: &BlockchainState,
        block: &Block,
        first_view_number: u32,
        txn: &mut WriteTransaction,
    ) -> Result<(), PushError> {
        // Check transactions against replay attacks. This is only necessary for micro blocks.
        if block.is_micro() {
            let transactions = block.transactions();

            if let Some(tx_vec) = transactions {
                for transaction in tx_vec {
                    if self.contains_tx_in_validity_window(&transaction.hash(), Some(txn)) {
                        warn!("Rejecting block - transaction already included");
                        return Err(PushError::DuplicateTransaction);
                    }
                }
            }
        }

        // Commit block to AccountsTree.
        if let Err(e) = self.commit_accounts(state, block, first_view_number, txn) {
            warn!("Rejecting block - commit failed: {:?}", e);
            #[cfg(feature = "metrics")]
            self.metrics.note_invalid_block();
            return Err(e);
        }

        // Verify the state against the block.
        if let Err(e) = self.verify_block_state(state, block, Some(txn)) {
            warn!("Rejecting block - Bad state");
            return Err(e);
        }

        Ok(())
    }
}
