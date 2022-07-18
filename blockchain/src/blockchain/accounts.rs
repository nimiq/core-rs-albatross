use nimiq_account::Accounts;
use nimiq_account::BlockLog;
use nimiq_block::{Block, MicroBlock, ViewChanges};
use nimiq_database::WriteTransaction;
use nimiq_primitives::policy;
use nimiq_vrf::VrfEntropy;

use crate::blockchain_state::BlockchainState;
use crate::history::ExtendedTransaction;
use crate::{Blockchain, PushError};

/// Implements methods to handle the accounts.
impl Blockchain {
    /// Updates the accounts given a block.
    pub fn commit_accounts(
        &self,
        state: &BlockchainState,
        block: &Block,
        prev_entropy: VrfEntropy,
        first_view_number: u32,
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
                    None,
                    macro_block.header.block_number,
                    macro_block.header.timestamp,
                    vec![],
                    inherents,
                );

                self.history_store.add_to_history(
                    txn,
                    policy::epoch_at(macro_block.header.block_number),
                    &ext_txs,
                );

                let batch_info = batch_info.unwrap();
                Ok(BlockLog::AppliedBlock {
                    inherent_logs: batch_info.inherent_logs,
                    block_hash: macro_block.hash(),
                    block_number: macro_block.header.block_number,
                    timestamp: macro_block.header.timestamp,
                    tx_logs: batch_info.tx_logs,
                })
            }
            Block::Micro(ref micro_block) => {
                // Get the body of the block.
                let body = micro_block.body.as_ref().unwrap();

                // Get the view changes.
                let view_changes = ViewChanges::new(
                    micro_block.header.block_number,
                    first_view_number,
                    micro_block.header.view_number,
                    prev_entropy,
                );

                // Create the inherents from any forks and view changes.
                let inherents =
                    self.create_slash_inherents(&body.fork_proofs, &view_changes, Some(txn));

                // Commit block to AccountsTree and create the receipts.
                let batch_info = accounts.commit(
                    txn,
                    &body.transactions,
                    &inherents,
                    micro_block.header.block_number,
                    micro_block.header.timestamp,
                );
                let batch_info = match batch_info {
                    Ok(batch_info) => batch_info,
                    Err(e) => {
                        // Check if the receipts contain an error.
                        return Err(PushError::AccountsError(e));
                    }
                };

                // Store receipts.
                let receipts = batch_info.receipts.into();
                self.chain_store
                    .put_receipts(txn, micro_block.header.block_number, &receipts);

                // Store the transactions and the inherents into the History tree.
                let ext_txs = ExtendedTransaction::from(
                    None,
                    micro_block.header.block_number,
                    micro_block.header.timestamp,
                    body.transactions.clone(),
                    inherents,
                );

                self.history_store.add_to_history(
                    txn,
                    policy::epoch_at(micro_block.header.block_number),
                    &ext_txs,
                );

                Ok(BlockLog::AppliedBlock {
                    inherent_logs: batch_info.inherent_logs,
                    block_hash: micro_block.hash(),
                    block_number: micro_block.header.block_number,
                    timestamp: micro_block.header.timestamp,
                    tx_logs: batch_info.tx_logs,
                })
            }
        }
    }

    /// Reverts the accounts given a block. This only applies to micro blocks, since macro blocks
    /// are final and can't be reverted.
    pub(crate) fn revert_accounts(
        &self,
        accounts: &Accounts,
        txn: &mut WriteTransaction,
        micro_block: &MicroBlock,
        prev_entropy: VrfEntropy,
        prev_view_number: u32,
    ) -> Result<BlockLog, PushError> {
        assert_eq!(
            micro_block.header.state_root,
            accounts.get_root(Some(txn)),
            "Failed to revert - inconsistent state"
        );

        debug!(
            block_number = &micro_block.header.block_number,
            view_number = &micro_block.header.view_number,
            "Reverting block"
        );

        // Get the body of the block.
        let body = micro_block.body.as_ref().unwrap();

        // Get the view changes.
        let view_changes = ViewChanges::new(
            micro_block.header.block_number,
            prev_view_number,
            micro_block.header.view_number,
            prev_entropy,
        );

        // Create the inherents from any forks and view changes.
        let inherents = self.create_slash_inherents(&body.fork_proofs, &view_changes, Some(txn));

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

        self.history_store.remove_partial_history(
            txn,
            policy::epoch_at(micro_block.header.block_number),
            num_txs,
        );

        Ok(BlockLog::RevertedBlock {
            inherent_logs: batch_info.inherent_logs,
            block_hash: micro_block.hash(),
            block_number: micro_block.header.block_number,
            tx_logs: batch_info.tx_logs,
        })
    }
}
