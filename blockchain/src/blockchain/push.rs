use std::ops::Deref;

use parking_lot::{RwLockUpgradableReadGuard, RwLockWriteGuard};

use nimiq_block::{Block, BlockType, ForkProof};
use nimiq_database::{ReadTransaction, WriteTransaction};
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_primitives::policy;

use crate::blockchain_state::BlockchainState;
use crate::chain_info::ChainInfo;
use crate::{
    AbstractBlockchain, Blockchain, BlockchainEvent, ChainOrdering, ForkEvent, PushError,
    PushResult,
};

/// Implements methods to push blocks into the chain. This is used when the node has already synced
/// and is just receiving newly produced blocks. It is also used for the final phase of syncing,
/// when the node is just receiving micro blocks.
impl Blockchain {
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
        // TODO: We might want to pass this as argument to this method.
        let read_txn = ReadTransaction::new(&this.env);

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

        // Calculate chain ordering.
        let chain_order =
            ChainOrdering::order_chains(this.deref(), &block, &prev_info, Some(&read_txn));

        // If it is an inferior chain, we ignore it as it cannot become better at any point in time.
        if chain_order == ChainOrdering::Inferior {
            info!(
                "Ignoring block - inferior chain (#{}, {})",
                block.block_number(),
                block.hash()
            );
            return Ok(PushResult::Ignored);
        }

        // Get the intended slot owner.
        let (validator, _) = this
            .get_slot_owner_at(block.block_number(), block.view_number(), Some(&read_txn))
            .expect("Couldn't calculate slot owner!");

        let intended_slot_owner = validator.public_key.uncompress_unchecked();

        // Check the header.
        if let Err(e) = Blockchain::verify_block_header(
            this.deref(),
            &block.header(),
            &intended_slot_owner,
            Some(&read_txn),
        ) {
            warn!("Rejecting block - Bad header");
            return Err(e);
        }

        // Check the justification.
        if let Err(e) = Blockchain::verify_block_justification(
            &*this,
            &block.header(),
            &block.justification(),
            &intended_slot_owner,
            Some(&read_txn),
        ) {
            warn!("Rejecting block - Bad justification");
            return Err(e);
        }

        // Check the body.
        if let Err(e) = this.verify_block_body(&block.header(), &block.body(), Some(&read_txn)) {
            warn!("Rejecting block - Bad body");
            return Err(e);
        }

        // Detect forks.
        if let Block::Micro(ref micro_block) = block {
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
                // If there's another micro block set to this view number, we
                // notify the fork event.
                if view_number == micro_block.header.view_number {
                    let micro_header2 = micro_block.header;
                    let justification2 = micro_block
                        .justification
                        .as_ref()
                        .expect("Missing justification!")
                        .signature;

                    let proof = ForkProof {
                        header1: micro_header1.clone(),
                        header2: micro_header2,
                        justification1: *justification1,
                        justification2,
                    };

                    this.fork_notifier.notify(ForkEvent::Detected(proof));
                }
            }
        }

        let chain_info = match ChainInfo::from_block(block, &prev_info) {
            Ok(info) => info,
            Err(err) => {
                warn!("Rejecting block - slash commit failed: {:?}", err);
                return Err(PushError::InvalidSuccessor);
            }
        };

        // Drop read transaction before calling other functions.
        drop(read_txn);

        // More chain ordering.
        match chain_order {
            ChainOrdering::Extend => {
                return Blockchain::extend(this, chain_info.head.hash(), chain_info, prev_info);
            }
            ChainOrdering::Better => {
                return Blockchain::rebranch(this, chain_info.head.hash(), chain_info);
            }
            ChainOrdering::Inferior => unreachable!(),
            ChainOrdering::Unknown => {}
        }

        // Otherwise, we are creating/extending a fork. Store ChainInfo.
        debug!(
            "Creating/extending fork with block {}, block number #{}, view number {}",
            chain_info.head.hash(),
            chain_info.head.block_number(),
            chain_info.head.view_number()
        );

        let mut txn = WriteTransaction::new(&this.env);

        this.chain_store
            .put_chain_info(&mut txn, &chain_info.head.hash(), &chain_info, true);

        txn.commit();

        Ok(PushResult::Forked)
    }

    /// Extends the current main chain.
    fn extend(
        this: RwLockUpgradableReadGuard<Blockchain>,
        block_hash: Blake2bHash,
        mut chain_info: ChainInfo,
        mut prev_info: ChainInfo,
    ) -> Result<PushResult, PushError> {
        let env = this.env.clone();
        let mut txn = WriteTransaction::new(&env);

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

        let is_election_block = policy::is_election_block_at(this.block_number() + 1);

        let mut is_macro = false;

        // Upgrade the lock as late as possible
        let mut this = RwLockUpgradableReadGuard::upgrade(this);

        if let Block::Macro(ref macro_block) = chain_info.head {
            is_macro = true;
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
        txn.commit();

        // Downgrade the lock again as the nofity listeners might want to acquire read access themselves.
        let this = RwLockWriteGuard::downgrade(this);

        if is_macro {
            if is_election_block {
                this.notifier
                    .notify(BlockchainEvent::EpochFinalized(block_hash));
            } else {
                this.notifier.notify(BlockchainEvent::Finalized(block_hash));
            }
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
        let env = this.env.clone();
        let read_txn = ReadTransaction::new(&env);

        let mut fork_chain: Vec<(Blake2bHash, ChainInfo)> = vec![];

        let mut current: (Blake2bHash, ChainInfo) = (block_hash, chain_info);

        while !current.1.on_main_chain {
            // A fork can't contain a macro block. We already received that macro block, thus it must be on our
            // main chain.
            assert_eq!(
                current.1.head.ty(),
                BlockType::Micro,
                "Fork contains macro block"
            );

            let prev_hash = current.1.head.parent_hash().clone();

            let prev_info = this
                .chain_store
                .get_chain_info(&prev_hash, true, Some(&read_txn))
                .expect("Corrupted store: Failed to find fork predecessor while rebranching");

            fork_chain.push(current);

            current = (prev_hash, prev_info);
        }

        debug!(
            "Found common ancestor {} at height #{}, {} blocks up",
            current.0,
            current.1.head.block_number(),
            fork_chain.len()
        );

        // Revert AccountsTree & TransactionCache to the common ancestor state.
        let mut revert_chain: Vec<(Blake2bHash, ChainInfo)> = vec![];
        let mut ancestor = current;

        let mut write_txn = WriteTransaction::new(&env);

        current = (this.state.head_hash.clone(), this.state.main_chain.clone());

        // Check if ancestor is in current batch.
        if ancestor.1.head.block_number() < this.state.macro_info.head.block_number() {
            info!("Ancestor is in finalized epoch");
            return Err(PushError::InvalidFork);
        }

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
                        .get_chain_info(&prev_hash, true, Some(&read_txn))
                        .expect("Corrupted store: Failed to find main chain predecessor while rebranching");

                    this.revert_accounts(
                        &this.state.accounts,
                        &mut write_txn,
                        micro_block,
                        prev_info.head.view_number(),
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
            match fork_block.1.head {
                Block::Macro(_) => unreachable!(),
                Block::Micro(ref micro_block) => {
                    if let Err(e) = this.check_and_commit(
                        &this.state,
                        &fork_block.1.head,
                        prev_view_number,
                        &mut write_txn,
                    ) {
                        warn!("Failed to apply fork block while rebranching - {:?}", e);
                        write_txn.abort();

                        // Delete invalid fork blocks from store.
                        let mut write_txn = WriteTransaction::new(&env);

                        for block in vec![fork_block].into_iter().chain(fork_iter) {
                            this.chain_store.remove_chain_info(
                                &mut write_txn,
                                &block.0,
                                micro_block.header.block_number,
                            )
                        }

                        write_txn.commit();

                        return Err(PushError::InvalidFork);
                    }

                    prev_view_number = fork_block.1.head.next_view_number();
                }
            }
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

        // Upgrade the lock as late as possible
        let mut this = RwLockUpgradableReadGuard::upgrade(this);

        // Commit transaction & update head.
        this.chain_store.set_head(&mut write_txn, &fork_chain[0].0);

        this.state.main_chain = fork_chain[0].1.clone();

        this.state.head_hash = fork_chain[0].0.clone();

        write_txn.commit();

        let mut reverted_blocks = Vec::with_capacity(revert_chain.len());

        for (hash, chain_info) in revert_chain.into_iter().rev() {
            reverted_blocks.push((hash, chain_info.head));
        }

        let mut adopted_blocks = Vec::with_capacity(fork_chain.len());

        for (hash, chain_info) in fork_chain.into_iter().rev() {
            adopted_blocks.push((hash, chain_info.head));
        }

        let event = BlockchainEvent::Rebranched(reverted_blocks, adopted_blocks);

        // Downgrade the lock again as the nofity listeners might want to acquire read access themselves.
        let this = RwLockWriteGuard::downgrade(this);

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
                    if self.contains_tx_in_validity_window(&transaction.hash()) {
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
