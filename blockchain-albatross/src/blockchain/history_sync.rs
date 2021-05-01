use parking_lot::MutexGuard;

use nimiq_block_albatross::{Block, BlockError};
use nimiq_database::{ReadTransaction, WriteTransaction};
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_primitives::coin::Coin;
use nimiq_primitives::policy;

use crate::chain_info::ChainInfo;
use crate::history_store::{ExtTxData, ExtendedTransaction, HistoryStore};
use crate::{AbstractBlockchain, Blockchain, BlockchainEvent, PushError, PushResult};

/// Implements methods to push macro blocks into the chain when an history node is syncing. This
/// type of syncing is called history syncing. It works by having the node get all the election
/// macro blocks since genesis plus the last macro block (most likely it will be a checkpoint block,
/// but it might be an election block). For these macro blocks the node must also get the
/// corresponding history tree. When the macro blocks are synced, then the node gets all the micro
/// blocks in the current batch and pushes them normally.
/// Note that, when pushing the macro blocks, we rely on the assumption that they were produced by
/// honest validator sets (defined as having less than 1/3 malicious validators). Because of that
/// we don't actually check the validity of the blocks, we just perform the minimal amount of checks
/// necessary to verify that the given block is a successor of our current chain so far and that the
/// corresponding history tree is actually part of the block.
impl Blockchain {
    /// Pushes a macro block (election or checkpoint) into the chain during history sync. You should
    /// NOT provide micro blocks as input. You can push election blocks after checkpoint blocks and
    /// vice-versa.
    pub fn push_history_sync(
        &self,
        block: Block,
        ext_txs: &[ExtendedTransaction],
    ) -> Result<PushResult, PushError> {
        // Only one push operation at a time.
        let push_lock = self.push_lock.lock();

        // TODO: We might want to pass this as argument to this method
        let read_txn = ReadTransaction::new(&self.env);

        // Check that it is a macro block. We can't push micro blocks with this function.
        let macro_block = match block {
            Block::Macro(ref b) => b,
            Block::Micro(_) => {
                return Err(PushError::InvalidSuccessor);
            }
        };

        // Check the version
        if macro_block.header.version != policy::VERSION {
            return Err(PushError::InvalidBlock(BlockError::UnsupportedVersion));
        }

        // Check if we already know this block.
        if self
            .chain_store
            .get_chain_info(&macro_block.hash(), false, Some(&read_txn))
            .is_some()
        {
            return Ok(PushResult::Known);
        }

        // Get the chain info at the head of the current chain.
        let head = self
            .chain_store
            .get_head(Some(&read_txn))
            .ok_or(PushError::Orphan)?;

        let prev_info = self
            .chain_store
            .get_chain_info(&head, false, Some(&read_txn))
            .ok_or(PushError::Orphan)?;

        // Check that the head is a macro block. This has to be the case since we never push micro
        // blocks while we are syncing.
        assert!(prev_info.head.is_macro());

        // Check if we have this block's parent. The checks change depending if the last macro block
        // that we pushed was an election block or not.
        if policy::is_election_block_at(prev_info.head.block_number()) {
            // We only need to check that the parent election block of this block is the same as our
            // head block.
            if macro_block.header.parent_election_hash != prev_info.head.hash() {
                return Err(PushError::Orphan);
            }
        } else {
            // We need to check that this block and our head block have the same parent election
            // block and are in the correct order.
            if &macro_block.header.parent_election_hash
                != prev_info.head.parent_election_hash().unwrap()
                || macro_block.header.block_number <= prev_info.head.block_number()
            {
                return Err(PushError::Orphan);
            }
        }

        // Check the history root.
        let history_root = HistoryStore::get_root_from_ext_txs(ext_txs)
            .ok_or(PushError::InvalidBlock(BlockError::InvalidHistoryRoot))?;

        if block.history_root() != &history_root {
            warn!("Rejecting block - wrong history root");
            return Err(PushError::InvalidBlock(BlockError::InvalidHistoryRoot));
        }

        // Checks if the body exists.
        let body = macro_block
            .body
            .as_ref()
            .ok_or(PushError::InvalidBlock(BlockError::MissingBody))?;

        // Check the body root.
        if body.hash::<Blake2bHash>() != macro_block.header.body_root {
            warn!("Rejecting block - Header body hash doesn't match real body hash");
            return Err(PushError::InvalidBlock(BlockError::BodyHashMismatch));
        }

        // Checks if the justification exists.
        let justification = macro_block
            .justification
            .as_ref()
            .ok_or(PushError::InvalidBlock(BlockError::NoJustification))?;

        // Check the justification.
        if !justification.verify(
            macro_block.hash(),
            macro_block.header.block_number,
            &self.current_validators().unwrap(),
        ) {
            warn!("Rejecting block - macro block with bad justification");
            return Err(PushError::InvalidBlock(BlockError::InvalidJustification));
        }

        // Extend the chain with this block.
        self.extend_history_sync(block, ext_txs, prev_info, push_lock)
    }

    /// Extends the current chain with a macro block (election or checkpoint) during history sync.
    fn extend_history_sync(
        &self,
        block: Block,
        ext_txs: &[ExtendedTransaction],
        mut prev_info: ChainInfo,
        push_lock: MutexGuard<()>,
    ) -> Result<PushResult, PushError> {
        // Create a new database write transaction.
        let mut txn = WriteTransaction::new(&self.env);

        // Get the block hash.
        let block_hash = block.hash();

        // Calculate the cumulative transaction fees for the current batch. This is necessary to
        // create the chain info for the block.
        let mut cum_tx_fees = Coin::ZERO;

        let current_batch = policy::batch_at(block.block_number());

        for i in (0..ext_txs.len()).rev() {
            if policy::batch_at(ext_txs[i].block_number) != current_batch {
                break;
            }

            if let ExtTxData::Basic(tx) = &ext_txs[i].data {
                cum_tx_fees += tx.fee;
            }
        }

        // Create the chain info for the current block and store it.
        let chain_info = ChainInfo {
            on_main_chain: true,
            main_chain_successor: None,
            head: block.clone(),
            cum_tx_fees,
        };

        self.chain_store
            .put_chain_info(&mut txn, &block_hash, &chain_info, true);

        // Update the chain info for the previous block and store it.
        prev_info.main_chain_successor = Some(chain_info.head.hash());

        self.chain_store
            .put_chain_info(&mut txn, &prev_info.head.hash(), &prev_info, false);

        // Set the head of the chain store to the current block.
        self.chain_store.set_head(&mut txn, &block_hash);

        // Get a read transaction to the current state.
        let state = self.state.read();

        // Get the index for the first extended transaction that was not already added in past macro
        // blocks.
        let mut first_new_ext_tx = 0;

        for ext_tx in ext_txs {
            if ext_tx.block_number <= prev_info.head.block_number() {
                first_new_ext_tx += 1;
            } else {
                break;
            }
        }

        // Separate the extended transactions by block number and type.
        // We know it comes sorted because we already checked it against the history root and
        // extended transactions in the history tree come sorted by block number and type.
        // Ignore the extended transactions that were already added in past macro blocks.
        let mut block_numbers = vec![];
        let mut block_timestamps = vec![];
        let mut block_transactions = vec![];
        let mut block_inherents = vec![];
        let mut prev = 0;

        for ext_tx in ext_txs.iter().skip(first_new_ext_tx) {
            if ext_tx.block_number > prev {
                block_numbers.push(ext_tx.block_number);
                block_timestamps.push(ext_tx.block_time);
                block_transactions.push(vec![]);
                block_inherents.push(vec![]);
                prev = ext_tx.block_number;
            }

            match &ext_tx.data {
                ExtTxData::Basic(tx) => block_transactions.last_mut().unwrap().push(tx.clone()),
                ExtTxData::Inherent(tx) => block_inherents.last_mut().unwrap().push(tx.clone()),
            }
        }

        // Update the accounts tree, one block at a time.
        for i in 0..block_numbers.len() {
            // Commit block to AccountsTree and create the receipts.
            let receipts = state.accounts.commit(
                &mut txn,
                &block_transactions[i],
                &block_inherents[i],
                block_numbers[i],
                block_timestamps[i],
            );

            // Check if the receipts contain an error.
            if let Err(e) = receipts {
                warn!("Rejecting block - commit failed: {:?}", e);
                txn.abort();
                #[cfg(feature = "metrics")]
                self.metrics.note_invalid_block();
                return Err(PushError::AccountsError(e));
            }
        }

        // Macro blocks are final and receipts for the previous batch are no longer necessary
        // as rebranching across this block is not possible.
        self.chain_store.clear_receipts(&mut txn);

        // Store the new extended transactions into the History tree.
        self.history_store.add_to_history(
            &mut txn,
            policy::epoch_at(block.block_number()),
            &ext_txs[first_new_ext_tx..],
        );

        // Unwrap the block.
        let macro_block = block.unwrap_macro_ref();

        // Check if this block is an election block.
        let is_election_block = macro_block.is_election_block();

        // Get a write transaction to the current state.
        drop(state);
        let mut state = self.state.write();

        // Update the blockchain state.
        state.main_chain = chain_info.clone();
        state.head_hash = block_hash.clone();
        state.macro_info = chain_info;
        state.macro_head_hash = block_hash.clone();
        if is_election_block {
            state.election_head = macro_block.clone();
            state.election_head_hash = block_hash.clone();
            state.previous_slots = state.current_slots.take();
            state.current_slots = macro_block.get_validators();
        }

        // Give up database transactions and push lock before creating notifications.
        txn.commit();
        drop(state);
        drop(push_lock);

        if is_election_block {
            self.notifier
                .read()
                .notify(BlockchainEvent::EpochFinalized(block_hash));
        } else {
            self.notifier
                .read()
                .notify(BlockchainEvent::Finalized(block_hash));
        }

        // Return result.
        Ok(PushResult::Extended)
    }
}
