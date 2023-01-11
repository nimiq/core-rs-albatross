use crate::blockchain_state::BlockchainState;
use crate::history::ExtendedTransaction;
use crate::Blockchain;
use nimiq_account::Accounts;
use nimiq_account::BlockLog;
use nimiq_block::{Block, BlockError::TransactionExecutionMismatch, SkipBlockInfo};
use nimiq_blockchain_interface::PushError;
use nimiq_database::WriteTransaction;
use nimiq_primitives::policy::Policy;

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

        // Check the type of the block.
        match block {
            Block::Macro(ref macro_block) => {
                // Initialize a vector to store the inherents
                let inherents = self.create_macro_block_inherents(state, &macro_block.header);

                // Commit block to AccountsTree and create the receipts.
                let batch_info = accounts.commit(
                    txn,
                    &[],
                    &inherents,
                    macro_block.header.block_number,
                    macro_block.header.timestamp,
                );

                // Check if the receipts contain an error.
                if let Err(e) = batch_info {
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

                let hs_result = self.history_store.add_to_history(
                    txn,
                    Policy::epoch_at(macro_block.header.block_number),
                    &ext_txs,
                );

                let (batch_info, _) = batch_info.unwrap();
                let (_, total_tx_size) = hs_result.unwrap();
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
                let batch_info = accounts.commit(
                    txn,
                    &body.get_raw_transactions(),
                    &inherents,
                    micro_block.header.block_number,
                    micro_block.header.timestamp,
                );
                let (batch_info, executed_txns) = match batch_info {
                    Ok(batch_info) => batch_info,
                    Err(e) => {
                        // Check if the receipts contain an error.
                        return Err(PushError::AccountsError(e));
                    }
                };

                // Check the executed transactions result obtained from the accounts commit against the ones in the block
                for (index, executed_txn) in executed_txns.iter().enumerate() {
                    if *executed_txn != body.transactions[index] {
                        return Err(PushError::InvalidBlock(TransactionExecutionMismatch));
                    }
                }

                // Store receipts.
                let receipts = batch_info.receipts.into();
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

                let hs_result = self.history_store.add_to_history(
                    txn,
                    Policy::epoch_at(micro_block.header.block_number),
                    &ext_txs,
                );

                let total_tx_size = hs_result.unwrap().1;
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
                assert_eq!(
                    micro_block.header.state_root,
                    accounts.get_root(Some(txn)),
                    "Failed to revert - inconsistent state"
                );

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
                let batch_info = accounts.revert(
                    txn,
                    &body.transactions,
                    &inherents,
                    micro_block.header.block_number,
                    micro_block.header.timestamp,
                    &receipts,
                );
                let batch_info = match batch_info {
                    Ok(batch_info) => batch_info,
                    Err(e) => {
                        panic!("Failed to revert - {:?}", e);
                    }
                };

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
