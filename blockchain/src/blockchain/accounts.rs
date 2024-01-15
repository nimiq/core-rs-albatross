use nimiq_account::{
    Account, Accounts, BlockLogger, BlockState, RevertInfo, TransactionOperationReceipt,
};
use nimiq_block::{Block, BlockError, SkipBlockInfo};
use nimiq_blockchain_interface::PushError;
use nimiq_database::{traits::Database, TransactionProxy};
use nimiq_keys::Address;
use nimiq_primitives::{
    key_nibbles::KeyNibbles,
    trie::{error::IncompleteTrie, trie_diff::TrieDiff, trie_proof::TrieProof},
};
use nimiq_serde::Deserialize;
use nimiq_transaction::historic_transaction::HistoricTransaction;
use nimiq_trie::WriteTransactionProxy;

use crate::{blockchain_state::BlockchainState, Blockchain};

/// Subset of the accounts in the accounts tree
pub struct AccountsChunk {
    /// The end of the chunk. The end key is exclusive.
    /// When set to None it means that it is the last trie chunk.
    pub end_key: Option<KeyNibbles>,
    /// The set of accounts retrieved.
    pub accounts: Vec<(Address, Account)>,
}

/// Implements methods to handle the accounts.
impl Blockchain {
    /// Updates the accounts given a block.
    /// Expects a full block with body.
    pub fn commit_accounts(
        &self,
        state: &BlockchainState,
        block: &Block,
        diff: Option<TrieDiff>,
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

                // Commit block to AccountsTree.
                if accounts.is_complete(Some(txn)) {
                    accounts.commit(txn, &[], &inherents, &block_state, block_logger)?;
                } else if let Some(diff) = diff {
                    accounts.commit_incomplete(txn, diff)?;
                } else {
                    return Err(PushError::MissingAccountsTrieDiff);
                }

                // Macro blocks are final and receipts for the previous batch are no longer necessary
                // as rebranching across this block is not possible.
                self.chain_store.clear_revert_infos(txn.raw());

                // Store the transactions and the inherents into the History tree.
                let hist_txs = HistoricTransaction::from(
                    self.network_id,
                    macro_block.header.block_number,
                    macro_block.header.timestamp,
                    vec![],
                    inherents,
                    vec![],
                );
                let total_tx_size = self
                    .history_store
                    .add_to_history(txn.raw(), macro_block.epoch_number(), &hist_txs)
                    .expect("Failed to store history")
                    .1;

                Ok(total_tx_size)
            }
            Block::Micro(ref micro_block) => {
                // Get the body of the block.
                let body = micro_block
                    .body
                    .as_ref()
                    .expect("Block body must be present");

                let skip_block_info = SkipBlockInfo::from_micro_block(micro_block);

                // Create the inherents from any equivocation proof or skip block info.
                let inherents = self.create_punishment_inherents(
                    block_state.number,
                    &body.equivocation_proofs,
                    skip_block_info,
                    Some(txn),
                );

                // Commit block to AccountsTree and create the receipts.
                let revert_info: RevertInfo = if accounts.is_complete(Some(txn)) {
                    accounts
                        .commit(
                            txn,
                            &body.get_raw_transactions(),
                            &inherents,
                            &block_state,
                            block_logger,
                        )?
                        .into()
                } else if let Some(diff) = diff {
                    accounts.commit_incomplete(txn, diff)?.into()
                } else {
                    return Err(PushError::MissingAccountsTrieDiff);
                };

                // Check that the transaction results match the ones in the block.
                if let RevertInfo::Receipts(receipts) = &revert_info {
                    assert_eq!(receipts.transactions.len(), body.transactions.len());
                    for (index, receipt) in receipts.transactions.iter().enumerate() {
                        let matches = match receipt {
                            TransactionOperationReceipt::Ok(..) => {
                                body.transactions[index].succeeded()
                            }
                            TransactionOperationReceipt::Err(..) => {
                                body.transactions[index].failed()
                            }
                        };
                        if !matches {
                            return Err(PushError::InvalidBlock(
                                BlockError::TransactionExecutionMismatch,
                            ));
                        }
                    }
                }

                // Store revert info.
                self.chain_store.put_revert_info(
                    txn.raw(),
                    micro_block.header.block_number,
                    &revert_info,
                );

                // Store the transactions and the inherents into the History tree.
                let hist_txs = HistoricTransaction::from(
                    self.network_id,
                    micro_block.header.block_number,
                    micro_block.header.timestamp,
                    body.transactions.clone(),
                    inherents,
                    body.equivocation_proofs
                        .iter()
                        .map(|proof| proof.locator())
                        .collect(),
                );
                let total_tx_size = self
                    .history_store
                    .add_to_history(txn.raw(), micro_block.epoch_number(), &hist_txs)
                    .expect("Failed to store history")
                    .1;

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
            num_equivocation_proofs = body.equivocation_proofs.len(),
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

        // Create the inherents from any equivocation proof or skip block info.
        let skip_block_info = SkipBlockInfo::from_micro_block(block);
        let inherents = self.create_punishment_inherents(
            block.block_number(),
            &body.equivocation_proofs,
            skip_block_info,
            Some(txn),
        );

        // Get the revert info for this block.
        let revert_info = self
            .chain_store
            .get_revert_info(block.block_number(), Some(txn))
            .expect("Failed to revert - missing revert info");

        // Revert the block from AccountsTree.
        let block_state = BlockState::new(block.block_number(), block.header.timestamp);
        let result = accounts.revert(
            txn,
            &body.get_raw_transactions(),
            &inherents,
            &block_state,
            revert_info,
            block_logger,
        );
        if let Err(e) = result {
            panic!("Failed to revert {block} - {e:?}");
        }

        // Remove the transactions from the History tree. For this you only need to calculate the
        // number of transactions that you want to remove.
        let num_txs = HistoricTransaction::count(
            body.transactions.len(),
            &inherents,
            body.equivocation_proofs
                .iter()
                .map(|proof| proof.locator())
                .collect(),
        );
        let (_, total_size) = self
            .history_store
            .remove_partial_history(txn.raw(), block.epoch_number(), num_txs)
            .expect("Failed to remove partial history");

        Ok(total_size)
    }

    /// Produces a Merkle proof of the inclusion of the given keys in the
    /// Merkle Radix Trie.
    pub fn get_accounts_proof(&self, keys: Vec<&KeyNibbles>) -> Result<TrieProof, IncompleteTrie> {
        let txn = self.env.read_transaction();

        self.state().accounts.get_proof(Some(&txn), keys)
    }

    /// Gets an accounts chunk given a start key and a limit
    pub fn get_accounts_chunk(
        &self,
        txn_option: Option<&TransactionProxy>,
        start: KeyNibbles,
        limit: usize,
    ) -> AccountsChunk {
        let trie_chunk = self.state().accounts.get_chunk(start, limit, txn_option);
        let end_key = trie_chunk.end_key;
        let accounts = trie_chunk
            .items
            .into_iter()
            .filter(|item| item.key.to_address().is_some())
            .map(|item| {
                (
                    item.key.to_address().unwrap(),
                    Account::deserialize_from_vec(&item.value).unwrap(),
                )
            })
            .collect();
        AccountsChunk { end_key, accounts }
    }
}
