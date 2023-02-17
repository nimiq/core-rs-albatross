use crate::blockchain_state::BlockchainState;
use crate::Blockchain;
use nimiq_account::{Accounts, BlockState};
use nimiq_account::{BlockLog, TransactionOperationReceipt};
use nimiq_block::{Block, BlockError, SkipBlockInfo};
use nimiq_blockchain_interface::PushError;
use nimiq_database::WriteTransaction;
use nimiq_primitives::policy::Policy;
use nimiq_transaction::extended_transaction::ExtendedTransaction;

/// Implements methods to handle the accounts.
impl Blockchain {
    /// Updates the accounts given a block.
    pub fn commit_accounts(
        &self,
        state: &BlockchainState,
        block: &Block,
        txn: &mut WriteTransaction,
    ) -> Result<BlockLog, PushError> {
        // Get the accounts from the state.
        let accounts = &state.accounts;
        let block_state = BlockState::new(block.block_number(), block.timestamp());

        // Check the type of the block.
        match block {
            Block::Macro(ref macro_block) => {
                // Initialize a vector to store the inherents
                let inherents = self.create_macro_block_inherents(macro_block);

                // Commit block to AccountsTree and create the receipts.
                let receipts = if accounts.is_complete(Some(txn)) {
                    accounts.commit(txn, &[], &inherents, &block_state)
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
                let ext_txs = ExtendedTransaction::from(
                    self.network_id,
                    macro_block.header.block_number,
                    macro_block.header.timestamp,
                    vec![],
                    inherents,
                );
                let total_tx_size = if state.can_verify_history {
                    let hs_result = self.history_store.add_to_history(
                        txn,
                        Policy::epoch_at(macro_block.header.block_number),
                        &ext_txs,
                    );
                    let (_, total_tx_size) = hs_result.unwrap();
                    total_tx_size
                } else {
                    0
                };

                let (batch_info, _) = batch_info.unwrap();
                Ok(BlockLog::AppliedBlock {
                    inherent_logs: batch_info.inherent_logs,
                    block_hash: macro_block.hash(),
                    block_number: macro_block.header.block_number,
                    timestamp: macro_block.header.timestamp,
                    tx_logs: batch_info.tx_logs,
                    total_tx_size,
                })
            }
            Block::Micro(ref micro_block) => {
                // Get the body of the block.
                let body = micro_block.body.as_ref().unwrap();

                let skip_block_info = if micro_block.is_skip_block() {
                    Some(SkipBlockInfo {
                        block_number: micro_block.header.block_number,
                        vrf_entropy: micro_block.header.seed.entropy(),
                    })
                } else {
                    None
                };

                // Create the inherents from any forks or skip block info.
                let inherents =
                    self.create_slash_inherents(&body.fork_proofs, skip_block_info, Some(txn));

                // Commit block to AccountsTree and create the receipts.
                let receipts = if accounts.is_complete(Some(txn)) {
                    accounts.commit(txn, &body.get_raw_transactions(), &inherents, &block_state)
                } else {
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
                        TransactionOperationReceipt::Err(_) => body.transactions[index].failed(),
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
                let ext_txs = ExtendedTransaction::from(
                    self.network_id,
                    micro_block.header.block_number,
                    micro_block.header.timestamp,
                    body.transactions.clone(),
                    inherents,
                );
                let total_tx_size = if state.can_verify_history {
                    let hs_result = self.history_store.add_to_history(
                        txn,
                        Policy::epoch_at(micro_block.header.block_number),
                        &ext_txs,
                    );
                    let (_, total_tx_size) = hs_result.unwrap();
                    total_tx_size
                } else {
                    0
                };

                Ok(BlockLog::AppliedBlock {
                    inherent_logs: batch_info.inherent_logs,
                    block_hash: micro_block.hash(),
                    block_number: micro_block.header.block_number,
                    timestamp: micro_block.header.timestamp,
                    tx_logs: batch_info.tx_logs,
                    total_tx_size,
                })
            }
        }
    }

    /// Reverts the accounts given a block. This only applies to micro blocks and skip blocks, since macro blocks
    /// are final and can't be reverted.
    pub(crate) fn revert_accounts(
        &self,
        accounts: &Accounts,
        txn: &mut WriteTransaction,
        block: &Block,
    ) -> Result<BlockLog, PushError> {
        match block {
            Block::Micro(ref micro_block) => {
                // Verify accounts hash if the tree is complete or changes only happened in the complete part.
                if let Some(accounts_hash) = accounts.get_root_hash(Some(txn)) {
                    assert_eq!(
                        micro_block.header.state_root, accounts_hash,
                        "Failed to revert - inconsistent state"
                    );
                }

                debug!(
                    block_number = &micro_block.header.block_number,
                    "Reverting block"
                );

                // Get the body of the block.
                let body = micro_block.body.as_ref().unwrap();

                let skip_block_info = if micro_block.is_skip_block() {
                    Some(SkipBlockInfo {
                        block_number: micro_block.header.block_number,
                        vrf_entropy: micro_block.header.seed.entropy(),
                    })
                } else {
                    None
                };

                // Create the inherents from any forks or skip block info.
                let inherents =
                    self.create_slash_inherents(&body.fork_proofs, skip_block_info, Some(txn));

                // Get the receipts for this block.
                let receipts = self
                    .chain_store
                    .get_receipts(micro_block.header.block_number, Some(txn))
                    .expect("Failed to revert - missing receipts");

                // Revert the block from AccountsTree.
                let block_state = BlockState::new(block.block_number(), block.timestamp());
                let result = accounts.revert(
                    txn,
                    &body.get_raw_transactions(),
                    &inherents,
                    &block_state,
                    receipts,
                );
                if let Err(e) = result {
                    panic!("Failed to revert - {e:?}");
                }

                // Remove the transactions from the History tree. For this you only need to calculate the
                // number of transactions that you want to remove.
                let num_txs = body.transactions.len() + inherents.len();

                let hs_result = self.history_store.remove_partial_history(
                    txn,
                    Policy::epoch_at(micro_block.header.block_number),
                    num_txs,
                );

                let (_, total_tx_size) = hs_result.unwrap();

                Ok(BlockLog::RevertedBlock {
                    inherent_logs: batch_info.inherent_logs,
                    block_hash: micro_block.hash(),
                    block_number: micro_block.header.block_number,
                    tx_logs: batch_info.tx_logs,
                    total_tx_size,
                })
            }
            Block::Macro(_) => unreachable!("Macro blocks are final and can't be reverted"),
        }
    }
}
