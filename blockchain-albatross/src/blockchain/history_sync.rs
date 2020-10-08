use parking_lot::MutexGuard;

use block::{Block, BlockError};
use database::{ReadTransaction, WriteTransaction};
use hash::{Blake2bHash, Hash};
use primitives::coin::Coin;
use primitives::policy;

use crate::chain_info::ChainInfo;
use crate::history_store::{ExtTxData, ExtendedTransaction, HistoryStore};
use crate::{Blockchain, BlockchainEvent, PushError, PushResult};

impl Blockchain {
    /// Pushes a macro block without requiring the micro blocks of the previous batch.
    // Not many checks, we only care about the signatures.
    pub fn push_history_sync(
        &self,
        block: Block,
        ext_txs: &[ExtendedTransaction],
    ) -> Result<PushResult, PushError> {
        // Only one push operation at a time.
        let push_lock = self.push_lock.lock();

        // TODO: We might want to pass this as argument to this method
        let read_txn = ReadTransaction::new(&self.env);

        // Check that it is a macro block.
        let macro_block = match block {
            Block::Macro(ref b) => b,
            Block::Micro(_) => {
                return Err(PushError::InvalidSuccessor);
            }
        };

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
                body.validators.as_ref().unwrap(),
                policy::TWO_THIRD_SLOTS,
            )
            .is_err()
        {
            warn!("Rejecting block - macro block with bad justification");
            return Err(PushError::InvalidBlock(BlockError::InvalidJustification));
        }

        self.extend_history_sync(block, ext_txs, prev_info, push_lock)
    }

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

        // Separate the extended transactions by type and block number.
        // We know it comes sorted because we already checked it against the history root.
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

        // Give up database transactions and push lock before creating notifiers.
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

        Ok(PushResult::Extended)
    }
}
