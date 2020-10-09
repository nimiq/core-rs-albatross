use parking_lot::MutexGuard;

use block::{Block, BlockError};
use database::{ReadTransaction, WriteTransaction};
use hash::{Blake2bHash, Hash};
use primitives::coin::Coin;
use primitives::policy;

use crate::chain_info::ChainInfo;
use crate::history_store::{ExtTxData, ExtendedTransaction, HistoryStore};
use crate::{Blockchain, BlockchainEvent, PushError, PushResult};

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
    /// NOT provide micro blocks as input. You cannot push any more blocks using this function after
    /// you push a checkpoint block.
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

        // Check that we didn't push any other macro block since the last election block. This is
        // necessary to guarantee that we don't push any more macro blocks after we push a
        // checkpoint block.
        if self.state.read().macro_head_hash != self.state.read().election_head_hash {
            return Err(PushError::InvalidSuccessor);
        }

        // Check if we already know this block.
        if self
            .chain_store
            .get_chain_info(&macro_block.hash(), false, Some(&read_txn))
            .is_some()
        {
            return Ok(PushResult::Known);
        }

        // Check if we have this block's parent. In this case we are not looking for the most direct
        // parent, but rather for the parent election block.
        let prev_info = self
            .chain_store
            .get_chain_info(
                &macro_block.header.parent_election_hash,
                false,
                Some(&read_txn),
            )
            .ok_or(PushError::Orphan)?;

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

        // Check the history root.
        let history_root = HistoryStore::root_from_ext_txs(ext_txs)
            .ok_or(PushError::InvalidBlock(BlockError::InvalidHistoryRoot))?;

        if body.history_root != history_root {
            warn!("Rejecting block - wrong history root");
            return Err(PushError::InvalidBlock(BlockError::InvalidHistoryRoot));
        }

        // Checks if the justification exists.
        let justification = macro_block
            .justification
            .as_ref()
            .ok_or(PushError::InvalidBlock(BlockError::NoJustification))?;

        // Check the justification.
        if justification
            .verify(
                macro_block.hash(),
                &self.current_validators(),
                policy::TWO_THIRD_SLOTS,
            )
            .is_err()
        {
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
        for i in (0..ext_txs.len()).rev() {
            if ext_txs[i].block_number != block.block_number() {
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
            .put_chain_info(&mut txn, &chain_info.head.hash(), &chain_info, true);

        // Update the chain info for the previous block and store it.
        prev_info.main_chain_successor = Some(chain_info.head.hash());

        self.chain_store
            .put_chain_info(&mut txn, &prev_info.head.hash(), &prev_info, false);

        // Set the head of the chain store to the current block.
        self.chain_store.set_head(&mut txn, &block_hash);

        // Get a read transaction to the current state.
        let state = self.state.read();

        // Separate the extended transactions by block number and type.
        // We know it comes sorted because we already checked it against the history root and
        // extended transactions in the history tree come sorted by block number and type.
        let mut block_numbers = vec![];
        let mut block_timestamps = vec![];
        let mut block_transactions = vec![];
        let mut block_inherents = vec![];
        let mut transactions = vec![];
        let mut inherents = vec![];
        let mut prev = 0;

        for ext_tx in ext_txs {
            if ext_tx.block_number > prev {
                prev = ext_tx.block_number;
                block_numbers.push(ext_tx.block_number);
                block_timestamps.push(ext_tx.block_time);
                block_transactions.push(transactions.clone());
                transactions.clear();
                block_inherents.push(inherents.clone());
                inherents.clear();
            }

            match &ext_tx.data {
                ExtTxData::Basic(tx) => transactions.push(tx.clone()),
                ExtTxData::Inherent(tx) => inherents.push(tx.clone()),
            }
        }

        // Update the accounts tree, one block at a time.
        for i in 0..block_numbers.len() {
            let receipts = state.accounts.commit(
                &mut txn,
                &block_transactions[i],
                &block_inherents[i],
                block_numbers[i],
                block_timestamps[i],
            );

            if let Err(e) = receipts {
                warn!("Rejecting block - commit failed: {:?}", e);
                txn.abort();
                #[cfg(feature = "metrics")]
                self.metrics.note_invalid_block();
                return Err(PushError::AccountsError(e));
            }
        }

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
            state.current_slots = macro_block.get_slots();
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
