use nimiq_account::Account;
use nimiq_blockchain::{AbstractBlockchain, HistoryTreeProof};
use nimiq_hash::Blake2bHash;
use nimiq_tree::accounts_tree_chunk::AccountsTreeChunk;

use crate::blockchain::NanoBlockchain;
use crate::error::NanoError;

/// Implements methods to verify data from the blocks, like accounts and transactions. Nano nodes
/// can request proofs of inclusion for given accounts or transactions and they can use these methods
/// to verify those proofs.
impl NanoBlockchain {
    /// Verify a Merkle proof for an account. It checks if the account is part of the Accounts Tree
    /// at the block with the given hash. It returns Ok if the proof is valid.
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
        if &account_proof.root() != block.state_root() {
            return Err(NanoError::WrongProof);
        }

        // Verify the account proof.
        if !account_proof.verify() {
            return Err(NanoError::WrongProof);
        }

        Ok(())
    }

    /// Verify a Merkle proof for a transaction. It checks if the transaction is part of the History
    /// Tree at the block with the given hash. It returns Ok if the proof is valid.
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
