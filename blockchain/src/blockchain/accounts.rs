use nimiq_account::{Accounts, BlockLogger, BlockState, TransactionOperationReceipt};
use nimiq_block::{Block, BlockError, SkipBlockInfo};
use nimiq_blockchain_interface::PushError;
use nimiq_database::{traits::Database, WriteTransactionProxy};
use nimiq_primitives::{key_nibbles::KeyNibbles, trie::trie_proof::TrieProof};
use nimiq_transaction::extended_transaction::ExtendedTransaction;

use crate::{blockchain_state::BlockchainState, Blockchain};

/// Implements methods to handle the accounts.
impl Blockchain {
    /// Updates the accounts given a block.
    /// Expects a full block with body.
    pub fn commit_accounts(
        &self,
        state: &BlockchainState,
        block: &Block,
        txn: &mut WriteTransactionProxy,
        block_logger: &mut BlockLogger,
    ) -> Result<u64, PushError> {
        // Get the accounts from the state.
        let accounts = &state.accounts;
        let block_state = BlockState::new(block.block_number(), block.timestamp());

        // Check the type of the block.
        match block {
            Block::Macro(ref macro_block) => {
                // Initialize a vector to store the inherents.
                let inherents = self.create_macro_block_inherents(macro_block);

                // Commit block to AccountsTree and create the receipts.
                let receipts = if accounts.is_complete(Some(txn)) {
                    accounts.commit(txn, &[], &inherents, &block_state, block_logger)
                } else {
                    accounts.commit_incomplete(txn, &[], &inherents, &block_state)
                };

                // Check if the receipts contain an error.
                if let Err(e) = receipts {
                    return Err(PushError::AccountsError(e));
                }

                // Macro blocks are final and receipts for the previous batch are no longer necessary
                // as rebranching across this block is not possible.
                self.chain_store.clear_receipts(txn);

                // Store the transactions and the inherents into the History tree.
                let mut total_tx_size = 0;
                if state.can_verify_history {
                    let ext_txs = ExtendedTransaction::from(
                        self.network_id,
                        macro_block.header.block_number,
                        macro_block.header.timestamp,
                        vec![],
                        inherents,
                    );
                    total_tx_size = self
                        .history_store
                        .add_to_history(txn, macro_block.epoch_number(), &ext_txs)
                        .expect("Failed to store history")
                        .1
                };

                Ok(total_tx_size)
            }
            Block::Micro(ref micro_block) => {
                // Get the body of the block.
                let body = micro_block
                    .body
                    .as_ref()
                    .expect("Block body must be present");

                let skip_block_info = SkipBlockInfo::from_micro_block(micro_block);

                // Create the inherents from any forks or skip block info.
                let inherents =
                    self.create_slash_inherents(&body.fork_proofs, skip_block_info, Some(txn));

                // Commit block to AccountsTree and create the receipts.
                let receipts = if accounts.is_complete(Some(txn)) {
                    accounts.commit(
                        txn,
                        &body.get_raw_transactions(),
                        &inherents,
                        &block_state,
                        block_logger,
                    )
                } else {
                    // We should not emit logs when syncing.
                    accounts.commit_incomplete(txn, &body.transactions, &inherents, &block_state)
                };

                // Check if the receipts contain an error.
                if let Err(e) = receipts {
                    return Err(PushError::AccountsError(e));
                }

                // Check that the transaction results match the ones in the block.
                let receipts = receipts.unwrap();
                assert_eq!(receipts.transactions.len(), body.transactions.len());
                for (index, receipt) in receipts.transactions.iter().enumerate() {
                    let matches = match receipt {
                        TransactionOperationReceipt::Ok(_) => body.transactions[index].succeeded(),
                        TransactionOperationReceipt::Err(..) => body.transactions[index].failed(),
                    };
                    if !matches {
                        return Err(PushError::InvalidBlock(
                            BlockError::TransactionExecutionMismatch,
                        ));
                    }
                }

                // Store receipts.
                self.chain_store
                    .put_receipts(txn, micro_block.header.block_number, &receipts);

                // Store the transactions and the inherents into the History tree.
                let mut total_tx_size = 0;
                if state.can_verify_history {
                    let ext_txs = ExtendedTransaction::from(
                        self.network_id,
                        micro_block.header.block_number,
                        micro_block.header.timestamp,
                        body.transactions.clone(),
                        inherents,
                    );
                    total_tx_size = self
                        .history_store
                        .add_to_history(txn, micro_block.epoch_number(), &ext_txs)
                        .expect("Failed to store history")
                        .1
                };

                Ok(total_tx_size)
            }
        }
    }

    /// Reverts the accounts given a block. This only applies to micro blocks and skip blocks, since
    /// macro blocks are final and can't be reverted.
    pub(crate) fn revert_accounts(
        &self,
        accounts: &Accounts,
        txn: &mut WriteTransactionProxy,
        block: &Block,
        block_logger: &mut BlockLogger,
    ) -> Result<u64, PushError> {
        if block.is_macro() {
            panic!("Can't revert {block} - macro blocks are final");
        }

        let block = block.unwrap_micro_ref();
        let body = block.body.as_ref().unwrap();

        debug!(
            block = %block,
            is_skip = block.is_skip_block(),
            num_transactions = body.transactions.len(),
            num_fork_proofs = body.fork_proofs.len(),
            "Reverting block"
        );

        // Verify accounts hash if the tree is complete or changes only happened in the complete part.
        if let Some(accounts_hash) = accounts.get_root_hash(Some(txn)) {
            assert_eq!(
                block.header.state_root, accounts_hash,
                "Cannot revert {} - inconsistent state",
                block,
            );
        }

        // Create the inherents from any forks or skip block info.
        let skip_block_info = SkipBlockInfo::from_micro_block(block);
        let inherents = self.create_slash_inherents(&body.fork_proofs, skip_block_info, Some(txn));

        // Get the receipts for this block.
        let receipts = self
            .chain_store
            .get_receipts(block.block_number(), Some(txn))
            .expect("Failed to revert - missing receipts");

        // Revert the block from AccountsTree.
        let block_state = BlockState::new(block.block_number(), block.header.timestamp);
        let result = accounts.revert(
            txn,
            &body.get_raw_transactions(),
            &inherents,
            &block_state,
            receipts,
            block_logger,
        );
        if let Err(e) = result {
            panic!("Failed to revert {block} - {e:?}");
        }

        // Remove the transactions from the History tree. For this you only need to calculate the
        // number of transactions that you want to remove.
        let num_txs = body.transactions.len() + inherents.len();
        let (_, total_size) = self
            .history_store
            .remove_partial_history(txn, block.epoch_number(), num_txs)
            .expect("Failed to remove partial history");

        Ok(total_size)
    }

    pub fn get_accounts_proof(&self, keys: Vec<&KeyNibbles>) -> Option<TrieProof> {
        let txn = self.env.read_transaction();

        self.state().accounts.tree.get_proof(&txn, keys).ok()
    }
}
