use nimiq_account::Account;
use nimiq_blockchain_albatross::{AbstractBlockchain, HistoryTreeProof};
use nimiq_hash::Blake2bHash;
use nimiq_tree_primitives::accounts_tree_chunk::AccountsTreeChunk;
use thiserror::Error;

use crate::blockchain::NanoBlockchain;

#[derive(Error, Debug, PartialEq, Eq)]
pub enum NanoError {
    #[error("Can't find block for account proof.")]
    MissingBlock,
    #[error("Block has no body.")]
    MissingBody,
    #[error("Block is either wrong type or has no body.")]
    InvalidBlock,
    #[error("Account proof root doesn't match block state root.")]
    StateRootMismatch,
    #[error("Account proof is wrong.")]
    WrongAccountProof,
}

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
            return Err(NanoError::StateRootMismatch);
        }

        // Verify the account proof.
        if !account_proof.verify() {
            return Err(NanoError::WrongAccountProof);
        }

        Ok(())
    }

    pub fn check_tx_history(
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
            return Err(NanoError::WrongAccountProof);
        }

        Ok(())
    }
}
