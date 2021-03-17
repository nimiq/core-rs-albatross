use nimiq_account::Account;
use nimiq_blockchain_albatross::{AbstractBlockchain, HistoryTreeProof};
use nimiq_hash::Blake2bHash;
use nimiq_tree_primitives::accounts_tree_chunk::AccountsTreeChunk;

use crate::blockchain::NanoBlockchain;
use crate::error::NanoError;

/// Implements methods to request data from the block, like accounts and transactions.
impl NanoBlockchain {
    pub fn check_account(
        &self,
        block_hash: Blake2bHash,
        mut account_proof: AccountsTreeChunk<Account>,
    ) -> Result<(), NanoError> {
        // Get the block.
        let block = self
            .get_block(&block_hash, false, None)
            .ok_or(NanoError::MissingBlock)?;

        // Check the root of the accounts proof against the state root.
        if !(&account_proof.root() == block.state_root()) {
            return Err(NanoError::WrongProof);
        }

        // Verify the account proof.
        if !account_proof.verify() {
            return Err(NanoError::WrongProof);
        }

        Ok(())
    }

    pub fn check_tx(
        &mut self,
        block_hash: Blake2bHash,
        tx_proof: HistoryTreeProof,
    ) -> Result<(), NanoError> {
        // Get the block.
        let block = self
            .get_block(&block_hash, false, None)
            .ok_or(NanoError::MissingBlock)?;

        // Verify the History Tree proof.
        if !tx_proof
            .verify(block.history_root().clone())
            .unwrap_or(false)
        {
            return Err(NanoError::WrongProof);
        }

        Ok(())
    }
}
