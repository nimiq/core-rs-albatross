use nimiq_account::Accounts;
use nimiq_block::{Block, MicroBlock, ViewChanges};
use nimiq_database::WriteTransaction;
use nimiq_primitives::policy;

use crate::blockchain_state::BlockchainState;
use crate::history_store::ExtendedTransaction;
use crate::{Blockchain, PushError};

/// Implements methods to handle the accounts.
impl Blockchain {
    /// Updates the accounts given a block.
    pub fn commit_accounts(
        &self,
        state: &BlockchainState,
        block: &Block,
        first_view_number: u32,
        txn: &mut WriteTransaction,
    ) -> Result<(), PushError> {
        // Get the accounts from the state.
        let accounts = &state.accounts;

        // Check the type of the block.
        match block {
            Block::Macro(ref macro_block) => {
                // Initialize a vector to store the inherents
                let inherents = self.create_macro_block_inherents(state, &macro_block.header);

                // Commit block to AccountsTree and create the receipts.
                let receipts = accounts.commit(
                    txn,
                    &[],
                    &inherents,
                    macro_block.header.block_number,
                    macro_block.header.timestamp,
                );

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

                self.history_store.add_to_history(
                    txn,
                    policy::epoch_at(macro_block.header.block_number),
                    &ext_txs,
                );
            }
            Block::Micro(ref micro_block) => {
                // Get the body of the block.
                let body = micro_block.body.as_ref().unwrap();

                // Get the view changes.
                let view_changes = ViewChanges::new(
                    micro_block.header.block_number,
                    first_view_number,
                    micro_block.header.view_number,
                );

                // Create the inherents from any forks and view changes.
                let inherents =
                    self.create_slash_inherents(&body.fork_proofs, &view_changes, Some(txn));

                // Commit block to AccountsTree and create the receipts.
                let receipts = accounts.commit(
                    txn,
                    &body.transactions,
                    &inherents,
                    micro_block.header.block_number,
                    micro_block.header.timestamp,
                );

                // Check if the receipts contain an error.
                if let Err(e) = receipts {
                    return Err(PushError::AccountsError(e));
                }

                // Store receipts.
                let receipts = receipts.unwrap();
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

                self.history_store.add_to_history(
                    txn,
                    policy::epoch_at(micro_block.header.block_number),
                    &ext_txs,
                );
            }
        }

        Ok(())
    }

    /// Reverts the accounts given a block. This only applies to micro blocks, since macro blocks
    /// are final and can't be reverted.
    pub(crate) fn revert_accounts(
        &self,
        accounts: &Accounts,
        txn: &mut WriteTransaction,
        micro_block: &MicroBlock,
        prev_view_number: u32,
    ) -> Result<(), PushError> {
        assert_eq!(
            micro_block.header.state_root,
            accounts.get_root(Some(txn)),
            "Failed to revert - inconsistent state"
        );

        // Get the body of the block.
        let body = micro_block.body.as_ref().unwrap();

        // Get the view changes.
        let view_changes = ViewChanges::new(
            micro_block.header.block_number,
            prev_view_number,
            micro_block.header.view_number,
        );

        // Create the inherents from any forks and view changes.
        let inherents = self.create_slash_inherents(&body.fork_proofs, &view_changes, Some(txn));

        // Get the receipts for this block.
        let receipts = self
            .chain_store
            .get_receipts(micro_block.header.block_number, Some(txn))
            .expect("Failed to revert - missing receipts");

        // Revert the block from AccountsTree.
        if let Err(e) = accounts.revert(
            txn,
            &body.transactions,
            &inherents,
            micro_block.header.block_number,
            micro_block.header.timestamp,
            &receipts,
        ) {
            panic!("Failed to revert - {:?}", e);
        }

        // Remove the transactions from the History tree. For this you only need to calculate the
        // number of transactions that you want to remove.
        let num_txs = body.transactions.len() + inherents.len();

        self.history_store.remove_partial_history(
            txn,
            policy::epoch_at(micro_block.header.block_number),
            num_txs,
        );

        Ok(())
    }
}
