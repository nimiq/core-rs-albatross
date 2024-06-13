use std::error::Error;

use nimiq_account::{BlockLog, BlockLogger};
use nimiq_blockchain_interface::{ChainInfo, PushError};
use nimiq_database::{TransactionProxy, WriteTransactionProxy};
use nimiq_hash::Blake2bHash;
use nimiq_primitives::trie::trie_diff::TrieDiff;

use crate::Blockchain;

impl Blockchain {
    /// Finds the common ancestor between the current main chain in the context of `txn` and the fork chain given by
    /// its chain info and the block hash.
    ///
    /// Returns the common ancestor as .0 and the chain leading to the common ancestor backwards from `block_hash` as .1
    /// The block with `block_hash` is included.
    pub(super) fn find_common_ancestor(
        &self,
        block_hash: Blake2bHash,
        chain_info: ChainInfo,
        diff: Option<TrieDiff>,
        txn: &TransactionProxy,
    ) -> Result<
        (
            (Blake2bHash, ChainInfo, Option<TrieDiff>),
            Vec<(Blake2bHash, ChainInfo, Option<TrieDiff>)>,
        ),
        PushError,
    > {
        // Walk up the fork chain until we find a block that is part of the main chain.
        // Store the chain along the way
        let target = chain_info.head.to_string();

        // Collects the chain on the way back to the common ancestor
        let mut fork_chain = vec![];

        // Keeps track of the currently investigated block
        let mut current: (Blake2bHash, ChainInfo, Option<TrieDiff>) =
            (block_hash, chain_info, diff);

        // Check if the currently checked block is on main chain. If so it is the common ancestor.
        while !current.1.on_main_chain {
            // If not keep on moving backwards so get the prev hash
            let prev_hash = current.1.head.parent_hash().clone();

            // Using the prev hash get the chain info of the previous block
            let prev_info = self
                .chain_store
                .get_chain_info(&prev_hash, true, Some(txn))
                .expect("Corrupted store: Failed to find fork predecessor while rebranching");

            // Get the prev diff for that block as well.
            let prev_diff = self
                .chain_store
                .get_accounts_diff(&prev_hash, Some(txn))
                .ok();

            // Store the current block as part of the fork
            fork_chain.push(current);

            // Update with the information regarding the previous block
            current = (prev_hash, prev_info, prev_diff);
        }

        // Check if ancestor is in current batch.
        if current.1.head.block_number() < self.state.macro_info.head.block_number() {
            warn!(
                block = target,
                reason = "ancestor block already finalized",
                ancestor_block = %current.1.head,
                "Rejecting block",
            );
            return Err(PushError::InvalidFork);
        }

        // Return the ancestor and the part of the chain used to get there.
        Ok((current, fork_chain))
    }

    /// Reverts all blocks until the common ancestor given as an argument is reached.
    /// After that applies all blocks given as target_chain in reverse order or until a block fails
    /// to be applied.
    ///
    /// Returns the reverted chain as `.1` and the block logs as `.2` or the blocks which are on a faulty fork.
    /// It does _not_ deal with the faulty blocks.
    pub(super) fn rebranch_to(
        &self,
        target_chain: &mut [(Blake2bHash, ChainInfo, Option<TrieDiff>)],
        ancestor: &mut (Blake2bHash, ChainInfo, Option<TrieDiff>),
        write_txn: &mut WriteTransactionProxy,
    ) -> Result<
        (Vec<(Blake2bHash, ChainInfo)>, Vec<BlockLog>),
        Vec<(Blake2bHash, ChainInfo, Option<TrieDiff>)>,
    > {
        // Keeps track of the currently investigated block
        let mut current = (self.state.head_hash.clone(), self.state.main_chain.clone());
        // Collects the reverted blocks
        let mut revert_chain: Vec<(Blake2bHash, ChainInfo)> = vec![];
        // Keep track of block logs
        let mut block_logs = vec![];

        // Start reverting blocks until the common ancestor is reached.
        while current.0 != ancestor.0 {
            let block = current.1.head.clone();

            // Macro blocks cannot be reverted.
            if block.is_macro() {
                panic!("Trying to rebranch across macro block {block}");
            }

            // Retrieve the predecessor for later use.
            let prev_hash = block.parent_hash().clone();
            let prev_info = self
                .chain_store
                .get_chain_info(&prev_hash, true, Some(write_txn))
                .expect("Corrupted store: Failed to find main chain predecessor while rebranching");

            // If previously a part of the accounts tree was missing the corresponding chunk must be reverted as well.
            if let Some(ref prev_missing_range) = current.1.prev_missing_range {
                self.state
                    .accounts
                    .revert_chunk(&mut write_txn.into(), prev_missing_range.start.clone())
                    .map_err(|error| {
                        warn!(
                            %block,
                            chain_info = ?current.1,
                            ?error,
                            "Failed to revert chunk while rebranching",
                        );
                        // The revert failed, but there are no blocks to remove.
                        vec![]
                    })?;
            }

            // Keep track of the logs for the upcoming revert
            let mut block_logger = BlockLogger::new_reverted(block.hash(), block.block_number());
            // Revert the accounts
            let total_tx_size = self
                .revert_accounts(
                    &self.state.accounts,
                    &mut write_txn.into(),
                    &block,
                    &mut block_logger,
                )
                .map_err(|error| {
                    warn!(
                        %block,
                        chain_info = ?current.1,
                        ?error,
                        "Failed to revert accounts while rebranching",
                    );
                    // The revert failed, but there are no blocks to remove.
                    vec![]
                })?;
            // Push the collected revert logs into the block logs collection.
            block_logs.push(block_logger.build(total_tx_size));

            // Verify accounts hash if the tree is complete or changes only happened in the complete part.
            if let Some(accounts_hash) = self.state.accounts.get_root_hash(Some(write_txn)) {
                assert_eq!(
                    prev_info.head.state_root(),
                    &accounts_hash,
                    "Inconsistent state after reverting block {} - {:?}",
                    block,
                    block,
                );
            }

            // Block was reverted, add it to the reverted chain collection.
            revert_chain.push(current);

            // Continue with the predecessor.
            current = (prev_hash, prev_info);
        }
        // Revert to common ancestor is done.

        // Next, push each block of the target chain.

        // Pushing must happen in reverse.
        let mut target_chain_iter = target_chain.iter().rev();

        while let Some(block) = target_chain_iter.next() {
            // Collect logs for the upcoming push.
            let mut block_logger = BlockLogger::new_applied(
                block.0.clone(),
                block.1.head.block_number(),
                block.1.head.timestamp(),
            );

            // Push the block
            match self.check_and_commit(
                &block.1.head,
                block.2.clone(),
                write_txn,
                &mut block_logger,
            ) {
                Ok(total_tx_size)
                    // push the logs into the logs collection
                    => block_logs.push(block_logger.build(total_tx_size)),
                Err(e) => {
                    // If a block fails to apply here it does not verify fully.
                    // This block and all blocks after this thus should be removed from the store
                    // as they are not verifying.
                    warn!(
                        block = %block.1.head,
                        reason = "failed to apply fork block while rebranching",
                        fork_block = %block.1.head,
                        error = &e as &dyn Error,
                        "Rejecting block",
                    );

                    // Since a write txn is open which cannot be closed the vec of the to-be-removed
                    // blocks is returned so the caller side can deal with it.
                    let remove_chain = vec![block.clone()]
                        .into_iter()
                        .chain(target_chain_iter.cloned())
                        .collect();
                    return Err(remove_chain);
                }
            }
        }

        // Unset on_main_chain flag / main_chain_successor on the current main chain up to (excluding) the common ancestor.
        for reverted_block in revert_chain.iter_mut() {
            reverted_block.1.on_main_chain = false;
            reverted_block.1.main_chain_successor = None;

            self.chain_store
                .put_chain_info(write_txn, &reverted_block.0, &reverted_block.1, false);
        }

        // Update the main_chain_successor of the common ancestor block.
        ancestor.1.main_chain_successor = Some(target_chain.last().unwrap().0.clone());
        self.chain_store
            .put_chain_info(write_txn, &ancestor.0, &ancestor.1, false);

        // Set on_main_chain flag / main_chain_successor on the fork.
        for i in (0..target_chain.len()).rev() {
            let main_chain_successor = if i > 0 {
                Some(target_chain[i - 1].0.clone())
            } else {
                None
            };

            let fork_block = &mut target_chain[i];
            fork_block.1.on_main_chain = true;
            fork_block.1.main_chain_successor = main_chain_successor;

            // Include the body of the new block (at position 0).
            self.chain_store
                .put_chain_info(write_txn, &fork_block.0, &fork_block.1, i == 0);
        }

        // Update the head.
        let new_head_hash = &target_chain[0].0;
        self.chain_store.set_head(write_txn, new_head_hash);

        Ok((revert_chain, block_logs))
    }
}
