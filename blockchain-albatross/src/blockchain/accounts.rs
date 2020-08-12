use account::Inherent;
use accounts::Accounts;
use block::{Block, MicroBlock, ViewChanges};
#[cfg(feature = "metrics")]
use blockchain_base::chain_metrics::BlockchainMetrics;
use database::WriteTransaction;

use crate::blockchain_state::BlockchainState;
use crate::chain_info::ChainInfo;
use crate::{Blockchain, PushError};

/// Everything to do with accounts
impl Blockchain {
    pub(crate) fn commit_accounts(
        &self,
        state: &BlockchainState,
        chain_info: &ChainInfo,
        first_view_number: u32,
        txn: &mut WriteTransaction,
    ) -> Result<(), PushError> {
        let block = &chain_info.head;

        let accounts = &state.accounts;

        match block {
            Block::Macro(ref macro_block) => {
                let mut inherents: Vec<Inherent> = vec![];

                // Every macro block is the end of a batch.
                inherents.append(
                    &mut self
                        .finalize_previous_batch(state, &chain_info.head.unwrap_macro_ref().header),
                );

                if macro_block.is_election_block() {
                    // On election the previous epoch needs to be finalized.
                    // We can rely on `state` here, since we cannot revert macro blocks.
                    inherents.push(self.finalize_previous_epoch());
                }

                // Commit block to AccountsTree.
                let receipts =
                    accounts.commit(txn, &[], &inherents, macro_block.header.block_number);

                // Macro blocks are final and receipts for the previous batch are no longer necessary
                // as rebranching across this block is not possible.
                self.chain_store.clear_receipts(txn);

                if let Err(e) = receipts {
                    return Err(PushError::AccountsError(e));
                }
            }
            Block::Micro(ref micro_block) => {
                let extrinsics = micro_block.body.as_ref().unwrap();

                let view_changes = ViewChanges::new(
                    micro_block.header.block_number,
                    first_view_number,
                    micro_block.header.view_number,
                );

                let inherents =
                    self.create_slash_inherents(&extrinsics.fork_proofs, &view_changes, Some(txn));

                // Commit block to AccountsTree.
                let receipts = accounts.commit(
                    txn,
                    &extrinsics.transactions,
                    &inherents,
                    micro_block.header.block_number,
                );

                if let Err(e) = receipts {
                    return Err(PushError::AccountsError(e));
                }

                // Store receipts.
                let receipts = receipts.unwrap();
                self.chain_store
                    .put_receipts(txn, micro_block.header.block_number, &receipts);
            }
        }

        Ok(())
    }

    pub(crate) fn revert_accounts(
        &self,
        accounts: &Accounts,
        txn: &mut WriteTransaction,
        micro_block: &MicroBlock,
        prev_view_number: u32,
    ) -> Result<(), PushError> {
        assert_eq!(
            micro_block.header.state_root,
            accounts.hash(Some(&txn)),
            "Failed to revert - inconsistent state"
        );

        let extrinsics = micro_block.body.as_ref().unwrap();

        let view_changes = ViewChanges::new(
            micro_block.header.block_number,
            prev_view_number,
            micro_block.header.view_number,
        );

        let inherents =
            self.create_slash_inherents(&extrinsics.fork_proofs, &view_changes, Some(txn));

        let receipts = self
            .chain_store
            .get_receipts(micro_block.header.block_number, Some(txn))
            .expect("Failed to revert - missing receipts");

        if let Err(e) = accounts.revert(
            txn,
            &extrinsics.transactions,
            &inherents,
            micro_block.header.block_number,
            &receipts,
        ) {
            panic!("Failed to revert - {}", e);
        }

        Ok(())
    }
}
