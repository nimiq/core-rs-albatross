use parking_lot::RwLockUpgradableReadGuard;

use nimiq_block_albatross::{Block, BlockType, ForkProof};
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
    /// Pushes a block into the chain.
    pub fn push(&self, block: Block) -> Result<PushResult, PushError> {
        // Only one push operation at a time.
        let _push_lock = self.push_lock.lock();

        // TODO: We might want to pass this as argument to this method.
        let read_txn = ReadTransaction::new(&self.env);

        // Check if we already know this block.
        if self
            .chain_store
            .get_chain_info(&block.hash(), false, Some(&read_txn))
            .is_some()
        {
            return Ok(PushResult::Known);
        }

        // Check if we have this block's parent.
        let prev_info = self
            .chain_store
            .get_chain_info(&block.parent_hash(), false, Some(&read_txn))
            .ok_or(PushError::Orphan)?;

        // Calculate chain ordering.
        let chain_order = ChainOrdering::order_chains(self, &block, &prev_info, Some(&read_txn));

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
        let (validator, _) = self
            .get_slot_owner_at(block.block_number(), block.view_number(), Some(&read_txn))
            .expect("Couldn't calculate slot owner!");

        let intended_slot_owner = validator.public_key.uncompress_unchecked();

        // Check the header.
        if let Err(e) = Blockchain::verify_block_header(
            self,
            &block.header(),
            &intended_slot_owner,
            Some(&read_txn),
        ) {
            warn!("Rejecting block - Bad header");
            return Err(e);
        }

        // Check the justification.
        if let Err(e) = Blockchain::verify_block_justification(
            self,
            &block.header(),
            &block.justification(),
            &intended_slot_owner,
            Some(&read_txn),
        ) {
            warn!("Rejecting block - Bad justification");
            return Err(e);
        }

        // Check the body.
        if let Err(e) = self.verify_block_body(&block.header(), &block.body(), Some(&read_txn)) {
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
                self.chain_store
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

                    self.fork_notifier.read().notify(ForkEvent::Detected(proof));
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
                return self.extend(chain_info.head.hash(), chain_info, prev_info);
            }
            ChainOrdering::Better => {
                return self.rebranch(chain_info.head.hash(), chain_info);
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

        let mut txn = WriteTransaction::new(&self.env);

        self.chain_store
            .put_chain_info(&mut txn, &chain_info.head.hash(), &chain_info, true);

        txn.commit();

        Ok(PushResult::Forked)
    }

    /// Extends the current main chain.
    fn extend(
        &self,
        block_hash: Blake2bHash,
        mut chain_info: ChainInfo,
        mut prev_info: ChainInfo,
    ) -> Result<PushResult, PushError> {
        let mut txn = WriteTransaction::new(&self.env);

        let state = self.state.read();

        if let Err(e) = self.check_and_commit(
            &state,
            &chain_info.head,
            prev_info.head.next_view_number(),
            &mut txn,
        ) {
            txn.abort();
            return Err(e);
        }

        drop(state);

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

        let is_election_block = policy::is_election_block_at(self.block_number() + 1);

        // Acquire write lock & commit changes.
        let mut state = self.state.write();

        let mut is_macro = false;
        if let Block::Macro(ref macro_block) = chain_info.head {
            is_macro = true;
            state.macro_info = chain_info.clone();
            state.macro_head_hash = block_hash.clone();

            if is_election_block {
                state.election_head = macro_block.clone();
                state.election_head_hash = block_hash.clone();

                let old_slots = state.current_slots.take().unwrap();
                state.previous_slots.replace(old_slots);

                let new_slots = macro_block.get_validators().unwrap();
                state.current_slots.replace(new_slots);
            }
        }

        state.main_chain = chain_info;
        state.head_hash = block_hash.clone();
        txn.commit();

        // Give up lock before notifying.
        drop(state);

        if is_macro {
            if is_election_block {
                self.notifier
                    .read()
                    .notify(BlockchainEvent::EpochFinalized(block_hash));
            } else {
                self.notifier
                    .read()
                    .notify(BlockchainEvent::Finalized(block_hash));
            }
        } else {
            self.notifier
                .read()
                .notify(BlockchainEvent::Extended(block_hash));
        }

        Ok(PushResult::Extended)
    }

    /// Rebranches the current main chain.
    fn rebranch(
        &self,
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
        let read_txn = ReadTransaction::new(&self.env);

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

            let prev_info = self
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

        let mut write_txn = WriteTransaction::new(&self.env);

        let state = self.state.upgradable_read();

        // TODO: Get rid of the .clone() here.
        current = (state.head_hash.clone(), state.main_chain.clone());

        // Check if ancestor is in current batch.
        if ancestor.1.head.block_number() < state.macro_info.head.block_number() {
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

                    let prev_info = self
                        .chain_store
                        .get_chain_info(&prev_hash, true, Some(&read_txn))
                        .expect("Corrupted store: Failed to find main chain predecessor while rebranching");

                    self.revert_accounts(
                        &state.accounts,
                        &mut write_txn,
                        &micro_block,
                        prev_info.head.view_number(),
                    )?;

                    assert_eq!(
                        prev_info.head.state_root(),
                        &state.accounts.hash(Some(&write_txn)),
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
                    if let Err(e) = self.check_and_commit(
                        &state,
                        &fork_block.1.head,
                        prev_view_number,
                        &mut write_txn,
                    ) {
                        warn!("Failed to apply fork block while rebranching - {:?}", e);
                        write_txn.abort();

                        // Delete invalid fork blocks from store.
                        let mut write_txn = WriteTransaction::new(&self.env);

                        for block in vec![fork_block].into_iter().chain(fork_iter) {
                            self.chain_store.remove_chain_info(
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

        // Fork looks good.
        // Drop read transaction.
        read_txn.close();

        // Acquire write lock.
        let mut state = RwLockUpgradableReadGuard::upgrade(state);

        // Unset onMainChain flag / mainChainSuccessor on the current main chain up to (excluding) the common ancestor.
        for reverted_block in revert_chain.iter_mut() {
            reverted_block.1.on_main_chain = false;

            reverted_block.1.main_chain_successor = None;

            self.chain_store.put_chain_info(
                &mut write_txn,
                &reverted_block.0,
                &reverted_block.1,
                false,
            );
        }

        // Update the mainChainSuccessor of the common ancestor block.
        ancestor.1.main_chain_successor = Some(fork_chain.last().unwrap().0.clone());
        self.chain_store
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
            self.chain_store
                .put_chain_info(&mut write_txn, &fork_block.0, &fork_block.1, i == 0);
        }

        // Commit transaction & update head.
        self.chain_store.set_head(&mut write_txn, &fork_chain[0].0);

        state.main_chain = fork_chain[0].1.clone();

        state.head_hash = fork_chain[0].0.clone();

        write_txn.commit();

        // Give up lock before notifying.
        drop(state);

        let mut reverted_blocks = Vec::with_capacity(revert_chain.len());

        for (hash, chain_info) in revert_chain.into_iter().rev() {
            reverted_blocks.push((hash, chain_info.head));
        }

        let mut adopted_blocks = Vec::with_capacity(fork_chain.len());

        for (hash, chain_info) in fork_chain.into_iter().rev() {
            adopted_blocks.push((hash, chain_info.head));
        }

        let event = BlockchainEvent::Rebranched(reverted_blocks, adopted_blocks);

        self.notifier.read().notify(event);

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
        if let Err(e) = self.commit_accounts(&state, &block, first_view_number, txn) {
            warn!("Rejecting block - commit failed: {:?}", e);
            #[cfg(feature = "metrics")]
            self.metrics.note_invalid_block();
            return Err(e);
        }

        // Verify the state against the block.
        if let Err(e) = self.verify_block_state(&state, &block, Some(&txn)) {
            warn!("Rejecting block - Bad state");
            return Err(e);
        }

        Ok(())
    }
}
