use std::error::Error;

use parking_lot::{RwLockUpgradableReadGuard, RwLockWriteGuard};

use nimiq_block::{Block, BlockError, MacroBlock, TendermintProof};
use nimiq_database::WriteTransaction;
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_primitives::coin::Coin;
use nimiq_primitives::policy;
use nimiq_transaction::Transaction;

use crate::chain_info::ChainInfo;
use crate::history::{ExtTxData, ExtendedTransaction};
use crate::{
    AbstractBlockchain, Blockchain, BlockchainEvent, HistoryTreeChunk, PushError, PushResult,
};
use nimiq_account::{Inherent, InherentType};

/// Struct used to identify extended transactions of a block
pub struct BlockTransactions {
    /// Block height or number
    pub block_number: u32,
    /// Timestamp of the block
    pub timestamp: u64,
    /// Transactions of the block
    pub transactions: Vec<Transaction>,
    /// Ineherents of the block
    pub inherents: Vec<Inherent>,
}

/// Struct produced after starting history sync.
/// The object provides methods for adding history chunks and finally commit
/// it to the history store.
pub struct HistoryPusher {
    /// Macro block for which the history chunks will be added
    pub block: MacroBlock,
    /// Number of transactions added to this HistoryPusher instance
    pub tx_count: u64,
    /// Cumulative transaction fees for the transactions added to this
    /// HistoryPusher instance
    pub cum_tx_fees: Coin,
    /// Last block transactions seen for a chunk
    pub last_transactions: Option<BlockTransactions>,
}

impl HistoryPusher {
    /// Commits a batch of transactions to the accounts tree for a set of blocks
    /// In case of error, the caller must abort the passed DB transaction.
    fn accounts_commit_batch(
        &mut self,
        blockchain: &RwLockUpgradableReadGuard<Blockchain>,
        txn: &mut WriteTransaction,
        chunk_block_txns: &mut [BlockTransactions],
    ) -> Result<PushResult, PushError> {
        let staking_contract_address = blockchain.staking_contract_address();

        // We go over the blocks one more time and add the FinalizeBatch and FinalizeEpoch inherents
        // to the macro blocks. This is necessary because the History Store doesn't store those inherents
        // so we need to add them again in order to correctly sync.
        for block_txns in chunk_block_txns.iter_mut() {
            if policy::is_macro_block_at(block_txns.block_number) {
                let finalize_batch = Inherent {
                    ty: InherentType::FinalizeBatch,
                    target: staking_contract_address.clone(),
                    value: Coin::ZERO,
                    data: vec![],
                };

                block_txns.inherents.push(finalize_batch);

                if policy::is_election_block_at(block_txns.block_number) {
                    let finalize_epoch = Inherent {
                        ty: InherentType::FinalizeEpoch,
                        target: staking_contract_address.clone(),
                        value: Coin::ZERO,
                        data: vec![],
                    };

                    block_txns.inherents.push(finalize_epoch);
                }
            }
        }

        // Update the accounts tree, one block at a time.
        for block_txns in chunk_block_txns {
            // Commit block to AccountsTree and create the receipts.
            let receipts = blockchain.state.accounts.commit_batch(
                txn,
                &block_txns.transactions,
                &block_txns.inherents,
                block_txns.block_number,
                block_txns.timestamp,
            );

            // Check if the receipts contain an error.
            if let Err(e) = receipts {
                warn!(
                    %self.block,
                    reason = "commit of macro block failed",
                    block_no = block_txns.block_number,
                    num_inherents = block_txns.inherents.len(),
                    error = &e as &dyn Error,
                    "Rejecting block",
                );

                #[cfg(feature = "metrics")]
                blockchain.metrics.note_invalid_block();
                return Err(PushError::AccountsError(e));
            }
        }
        blockchain.state.accounts.finalize_batch(txn);

        Ok(PushResult::Extended)
    }

    /// Adds transactions to the history pusher.
    /// Transactions are only received in chunks using the `HistoryTreeChunk`
    /// struct. This means that the chunks must come with a proof that relates
    /// them with this object macro block's history root.
    /// The proof is checked against the provided macro block's history root.
    /// The blockchain state may be changed if a need to reverse some micro
    /// blocks is detected in cases such as a partially known epoch or
    /// different adopted history.
    pub fn add_history_chunk(
        &mut self,
        blockchain: RwLockUpgradableReadGuard<Blockchain>,
        history_chunk: HistoryTreeChunk,
        chunk_index: usize,
        chunk_size: usize,
    ) -> Result<PushResult, PushError> {
        let macro_block = self.block.clone();

        // Check the history chunk proof against the macro block history root
        if !history_chunk
            .verify(macro_block.header.history_root, chunk_index * chunk_size)
            .unwrap_or(false)
        {
            log::warn!(
                "History Chunk failed to verify (chunk {} of epoch {})",
                chunk_index,
                self.block.epoch_number()
            );
            return Err(PushError::InvalidHistoryChunk);
        }

        // Create a new database write transaction.
        let mut txn = blockchain.write_transaction();

        // Get the block hash.
        let syncing_batch = policy::batch_at(self.block.block_number());

        // Calculate the cumulative transaction fees for the given batch. This is necessary to
        // create the chain info for the block.
        for i in (0..history_chunk.history.len()).rev() {
            if policy::batch_at(history_chunk.history[i].block_number) != syncing_batch {
                // We have reached the end of the batch we're interested in
                // We only need the same batch the macro block is at to calculate
                // the `cum_tx_fees` for later updating the blockchain `ChainInfo`
                break;
            }
            if let ExtTxData::Basic(tx) = &history_chunk.history[i].data {
                self.cum_tx_fees += tx.fee;
            }
        }

        // We might already know the given epoch partially.
        // Revert our chain to a common ancestor state in case we have adopted a different history.
        // Also skip over any transactions that we already know.
        let (current_chain_info, first_new_ext_tx_idx) =
            self.revert_to_common_state(&blockchain, &history_chunk.history, &mut txn);

        // Separate the extended transactions by block number and type.
        // We know it comes sorted because we already checked it against the history root and
        // extended transactions in the history tree come sorted by block number and type.
        // Ignore the extended transactions that were already added in past macro blocks.
        let (mut chunk_block_txns, mut prev) =
            if let Some(last_block_txns) = self.last_transactions.take() {
                let last_block_number = last_block_txns.block_number;
                (vec![last_block_txns], last_block_number)
            } else {
                (vec![], 0)
            };

        for ext_tx in history_chunk.history.iter().skip(first_new_ext_tx_idx) {
            if ext_tx.block_number > prev {
                let block_txns = BlockTransactions {
                    block_number: ext_tx.block_number,
                    timestamp: ext_tx.block_time,
                    transactions: vec![],
                    inherents: vec![],
                };
                chunk_block_txns.push(block_txns);
                prev = ext_tx.block_number;
            }

            match &ext_tx.data {
                ExtTxData::Basic(tx) => chunk_block_txns
                    .last_mut()
                    .unwrap()
                    .transactions
                    .push(tx.clone()),
                ExtTxData::Inherent(tx) => chunk_block_txns
                    .last_mut()
                    .unwrap()
                    .inherents
                    .push(tx.clone()),
            }
        }

        // Remove the last block's transactions and cache it in case it continues in the following
        // chunk
        self.last_transactions = chunk_block_txns.pop();

        // Try to commit accounts
        if let Err(e) = self.accounts_commit_batch(&blockchain, &mut txn, &mut chunk_block_txns) {
            txn.abort();
            return Err(e);
        }

        // Store the new extended transactions into the History tree.
        blockchain.history_store.add_to_history(
            &mut txn,
            self.block.epoch_number(),
            &history_chunk.history[first_new_ext_tx_idx..],
        );

        // Give up database transactions and push lock before creating notifications.
        txn.commit();

        // Update the blockchain state
        let mut blockchain = RwLockUpgradableReadGuard::upgrade(blockchain);
        blockchain.state.head_hash = current_chain_info.head.hash();
        blockchain.state.main_chain = current_chain_info;

        self.tx_count += history_chunk.history.len() as u64;

        // Return result.
        Ok(PushResult::Extended)
    }

    /// Commits the history pusher. This function performs the final checks to
    /// push the macro block provided and advance the blockchain state.
    /// This methods should only be called when no more history chunks are
    /// expected. A final check is also performed to verify that the accounts
    /// tree root matches the macro block's history root.
    pub fn commit(
        &mut self,
        blockchain: RwLockUpgradableReadGuard<Blockchain>,
    ) -> Result<PushResult, PushError> {
        let mut txn = blockchain.write_transaction();
        let block_hash = self.block.hash();
        let mut prev_macro_info = blockchain.state.macro_info.clone();
        let macro_block = self.block.clone();

        // Get the current macro head.
        let macro_head = &blockchain.state.macro_info.head;

        // Check (again) if we already know this block.
        if blockchain
            .chain_store
            .get_chain_info(&macro_block.hash(), false, Some(&txn))
            .is_some()
        {
            return Err(PushError::AlreadyKnown);
        }

        // Check if there is pending block transactions to push
        if let Some(last_account_txns) = self.last_transactions.take() {
            if let Err(e) =
                self.accounts_commit_batch(&blockchain, &mut txn, &mut [last_account_txns])
            {
                txn.abort();
                return Err(e);
            }
        }

        // Check if we have this block's parent. The checks change depending if the last macro block
        // that we pushed was an election block or not.
        if macro_head.is_election() {
            // We only need to check that the parent election block of this block is the same as our
            // head block.
            if macro_block.header.parent_election_hash != blockchain.state.macro_head_hash {
                warn!(
                    block = %macro_block,
                    reason = "wrong parent election hash",
                    parent_election_hash = %macro_block.header.parent_election_hash,
                    wanted_parent_election_hash = %blockchain.state.macro_head_hash,
                    "Rejecting block",
                );
                return Err(PushError::Orphan);
            }
        } else {
            // We need to check that this block and our head block have the same parent election
            // block and are in the correct order.
            let wanted_parent_election_hash = macro_head.parent_election_hash().unwrap();
            if macro_block.header.parent_election_hash != *wanted_parent_election_hash {
                warn!(
                    block = %macro_block,
                    reason = "wrong parent election hash",
                    parent_election_hash = %macro_block.header.parent_election_hash,
                    %wanted_parent_election_hash,
                    "Rejecting block",
                );
                return Err(PushError::Orphan);
            }

            // FIXME Compute the expected block number and compare it to the given one.
            if macro_block.header.block_number <= macro_head.block_number() {
                warn!(
                    block = %macro_block,
                    reason = "decreasing block number",
                    block_no = macro_block.header.block_number,
                    previous_block_no = macro_head.block_number(),
                    "Rejecting block",
                );
                return Err(PushError::AlreadyKnown);
            }
        }

        // Check the justification.
        if !TendermintProof::verify(&macro_block, &blockchain.current_validators().unwrap()) {
            warn!(
                block = %macro_block,
                reason = "bad justification",
                "Rejecting block",
            );
            return Err(PushError::InvalidBlock(BlockError::InvalidJustification));
        }

        // Create the chain info for the given block and store it.
        let chain_info = ChainInfo {
            on_main_chain: true,
            main_chain_successor: None,
            head: Block::Macro(macro_block.clone()),
            cum_tx_fees: self.cum_tx_fees,
        };

        blockchain
            .chain_store
            .put_chain_info(&mut txn, &block_hash, &chain_info, true);

        // Update the chain info for the previous macro block and store it.
        prev_macro_info.main_chain_successor = Some(chain_info.head.hash());

        blockchain.chain_store.put_chain_info(
            &mut txn,
            &prev_macro_info.head.hash(),
            &prev_macro_info,
            false,
        );

        // Set the head of the chain store to the current block.
        blockchain.chain_store.set_head(&mut txn, &block_hash);

        // Check the state_root hash against the one in the block.
        let wanted_state_root = blockchain.state.accounts.get_root(Some(&txn));
        if macro_block.header.state_root != wanted_state_root {
            warn!(
                block = %macro_block,
                reason = "header accounts hash doesn't match real accounts hash",
                state_root = %macro_block.header.state_root,
                %wanted_state_root,
                "Rejecting block",
            );
            txn.abort();
            #[cfg(feature = "metrics")]
            blockchain.metrics.note_invalid_block();
            return Err(PushError::InvalidBlock(BlockError::AccountsHashMismatch));
        }

        // Macro blocks are final and receipts for the previous batch are no longer necessary
        // as rebranching across this block is not possible.
        blockchain.chain_store.clear_receipts(&mut txn);

        // Give up database transactions and push lock before creating notifications.
        txn.commit();

        // Update the blockchain state.
        let mut blockchain = RwLockUpgradableReadGuard::upgrade(blockchain);
        blockchain.state.main_chain = chain_info.clone();
        blockchain.state.head_hash = block_hash.clone();
        blockchain.state.macro_info = chain_info;
        blockchain.state.macro_head_hash = block_hash.clone();

        // Check if this block is an election block.
        let is_election_block = macro_block.is_election_block();
        if is_election_block {
            blockchain.state.election_head = macro_block.clone();
            blockchain.state.election_head_hash = block_hash.clone();
            blockchain.state.previous_slots = blockchain.state.current_slots.take();
            blockchain.state.current_slots = macro_block.get_validators();
        }

        let blockchain = RwLockWriteGuard::downgrade_to_upgradable(blockchain);

        debug!(
            %macro_block,
            kind = "history_sync",
            "Accepted macro block",
        );

        debug!(
            epoch_no = macro_block.epoch_number(),
            num_items = self.tx_count,
            kind = "history_sync",
            "Accepted epoch",
        );

        if is_election_block {
            blockchain
                .notifier
                .notify(BlockchainEvent::EpochFinalized(block_hash));
        } else {
            blockchain
                .notifier
                .notify(BlockchainEvent::Finalized(block_hash));
        }

        Ok(PushResult::Extended)
    }

    /// Reverts the history store to a common state.
    /// This function is used to revert the chain to a common ancestor state
    /// in case a different history has been adopted.
    /// Also skip over any transactions that we already know.
    /// Note that this function doesn't change the blockchain state and leaves
    /// this responsibility to the caller who should update it based on the
    /// returned `ChainInfo`.
    fn revert_to_common_state(
        &self,
        blockchain: &RwLockUpgradableReadGuard<Blockchain>,
        history: &[ExtendedTransaction],
        txn: &mut WriteTransaction,
    ) -> (ChainInfo, usize) {
        // Find the index of the first extended transaction in the current batch.
        let last_macro_block = policy::last_macro_block(blockchain.block_number());
        // Get the chain info for the head of the chain.
        let mut current_info = blockchain
            .get_chain_info(&blockchain.head_hash(), true, Some(txn))
            .expect("Couldn't fetch chain info for the head of the chain!");
        let mut first_new_ext_tx_idx = history
            .iter()
            .position(|ext_tx| ext_tx.block_number > last_macro_block)
            .unwrap_or(history.len());

        // Check if our adopted non-final history matches the given history.
        // Revert any blocks that don't match.
        let known_history = blockchain
            .history_store
            .get_nonfinal_epoch_transactions(self.block.epoch_number(), Some(txn));
        if !known_history.is_empty() {
            // Iterate over the known history and the given history in parallel to find the block
            // where the histories diverge (if they do).
            let mut known = known_history.iter();
            let mut given = history.iter().skip(first_new_ext_tx_idx);
            let mut last_known_block = None;
            let diverging_block = loop {
                match (known.next(), given.next()) {
                    (Some(known_tx), Some(given_tx)) => {
                        last_known_block = Some(known_tx.block_number);
                        if *known_tx != *given_tx {
                            break Some(known_tx.block_number);
                        }
                    }
                    (None, Some(given_tx)) => {
                        break match last_known_block {
                            Some(block_number) if block_number == given_tx.block_number => {
                                Some(block_number)
                            }
                            _ => None,
                        };
                    }
                    (Some(known_tx), None) => break Some(known_tx.block_number),
                    (None, None) => break None,
                }
            };

            if let Some(diverging_block) = diverging_block {
                // The histories diverge, so revert our state to the block before the divergence.
                let num_blocks_to_revert = blockchain.block_number() - diverging_block + 1;
                current_info = Blockchain::revert_blocks(blockchain, num_blocks_to_revert, txn)
                    .expect("Failed to revert chain");

                // TODO We could incorporate this into the parallel iteration loop above.
                first_new_ext_tx_idx += history
                    .iter()
                    .skip(first_new_ext_tx_idx)
                    .position(|ext_tx| ext_tx.block_number >= diverging_block)
                    .unwrap_or(history.len() - first_new_ext_tx_idx);
            } else {
                // The histories match, so we can skip over all known transactions.
                first_new_ext_tx_idx += known_history.len();
            }
        } else if blockchain.state.main_chain.head.is_micro()
            && first_new_ext_tx_idx < history.len()
        {
            // We have micro blocks for the current batch but the known history is empty.
            // Check if the given history contains any items before our current block; if so, we
            // need to revert.
            let first_block_number = history[first_new_ext_tx_idx].block_number;
            if first_block_number <= blockchain.block_number() {
                let num_blocks_to_revert = blockchain.block_number() - first_block_number + 1;
                current_info = Blockchain::revert_blocks(blockchain, num_blocks_to_revert, txn)
                    .expect("Failed to revert chain");
            }
        };

        (current_info, first_new_ext_tx_idx)
    }
}

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
    /// Starts history sync for a specific macro block (election or checkpoint).
    /// This function performs basic checks on the macro block provided and
    /// then returns a `HistoryPusher` instance where history chunks can be
    /// added later.
    /// Note that this function doesn't alter the blockchain state nor pushes
    /// anything to the history store. These changes must be performed with the
    /// `HistoryPusher` instance.
    pub fn start_history_sync(
        this: RwLockUpgradableReadGuard<Self>,
        macro_block: MacroBlock,
    ) -> Result<HistoryPusher, PushError> {
        // Create a new database read transaction.
        let read_txn = this.read_transaction();

        // Check the version
        if macro_block.header.version != policy::VERSION {
            warn!(
                %macro_block,
                reason = "wrong version",
                version = macro_block.header.version,
                wanted_version = policy::VERSION,
                "Rejecting block",
            );
            return Err(PushError::InvalidBlock(BlockError::UnsupportedVersion));
        }

        // Check if we already know this block.
        if this
            .chain_store
            .get_chain_info(&macro_block.hash(), false, Some(&read_txn))
            .is_some()
        {
            return Err(PushError::AlreadyKnown);
        }

        // Checks if the body exists.
        let body = macro_block.body.as_ref().ok_or_else(|| {
            warn!(
                block = %macro_block,
                reason = "body missing",
                "Rejecting block"
            );
            PushError::InvalidBlock(BlockError::MissingBody)
        })?;

        // Check the body root.
        let body_hash = body.hash::<Blake2bHash>();
        if macro_block.header.body_root != body_hash {
            warn!(
                block = %macro_block,
                reason = "header body hash doesn't match real body hash",
                given_body_hash = %macro_block.header.body_root,
                actual_body_hash = %body_hash,
                "Rejecting block",
            );
            return Err(PushError::InvalidBlock(BlockError::BodyHashMismatch));
        }

        drop(read_txn);

        // Extend the chain with this block.
        //let prev_macro_info = this.state.macro_info.clone();
        //Blockchain::extend_history_sync(this, block, history, prev_macro_info)
        Ok(HistoryPusher {
            block: macro_block,
            tx_count: 0,
            cum_tx_fees: Coin::ZERO,
            last_transactions: None,
        })
    }

    /// Reverts a given number of micro blocks from the blockchain.
    /// This function updates the history store but doesn't alter the
    /// blockchain state. This function returns a `ChainInfo` struct such
    /// that the caller makes the proper state changes.
    pub fn revert_blocks(
        &self,
        num_blocks: u32,
        write_txn: &mut WriteTransaction,
    ) -> Result<ChainInfo, PushError> {
        debug!(
            num_blocks,
            "Need to revert micro blocks from the current epoch",
        );
        // Get the chain info for the head of the chain.
        let mut current_info = self
            .get_chain_info(&self.head_hash(), true, Some(write_txn))
            .expect("Couldn't fetch chain info for the head of the chain!");

        // Revert each block individually.
        for _ in 0..num_blocks {
            match current_info.head {
                Block::Micro(ref micro_block) => {
                    // Get the chain info for the parent of the current head of the chain.
                    let prev_info = self
                        .get_chain_info(&micro_block.header.parent_hash, true, Some(write_txn))
                        .expect("Failed to find main chain predecessor while reverting blocks!");

                    // Revert the accounts tree. This also reverts the history store.
                    self.revert_accounts(
                        &self.state.accounts,
                        write_txn,
                        micro_block,
                        prev_info.head.seed().entropy(),
                        prev_info.head.next_view_number(),
                    )?;

                    current_info = prev_info;
                }
                Block::Macro(_) => {
                    unreachable!();
                }
            }
        }

        // Update the blockchain
        let block_hash = current_info.head.hash();

        self.chain_store
            .put_chain_info(write_txn, &block_hash, &current_info, true);

        // Set the head of the chain store to the current block.
        self.chain_store.set_head(write_txn, &block_hash);

        Ok(current_info)
    }
}
