#[cfg(feature = "metrics")]
use std::sync::Arc;

use nimiq_account::{Account, StakingContract};
use nimiq_block::Block;
use nimiq_blockchain_interface::{AbstractBlockchain, BlockchainError, ChainInfo, Direction};
use nimiq_database::{ReadTransaction, Transaction};
use nimiq_hash::Blake2bHash;
use nimiq_keys::Address;
use nimiq_primitives::{key_nibbles::KeyNibbles, policy::Policy, slots::Validator};
use std::ops::RangeFrom;

#[cfg(feature = "metrics")]
use crate::chain_metrics::BlockchainMetrics;
use crate::{blockchain_state::BlockchainState, Blockchain};

/// Implements several wrapper functions.
impl Blockchain {
    /// Returns the current state
    pub fn state(&self) -> &BlockchainState {
        &self.state
    }

    pub fn get_block_at(
        &self,
        height: u32,
        include_body: bool,
        txn_option: Option<&Transaction>,
    ) -> Result<Block, BlockchainError> {
        self.chain_store
            .get_block_at(height, include_body, txn_option)
    }

    pub fn get_block(
        &self,
        hash: &Blake2bHash,
        include_body: bool,
        txn_option: Option<&Transaction>,
    ) -> Result<Block, BlockchainError> {
        self.chain_store.get_block(hash, include_body, txn_option)
    }

    pub fn get_blocks(
        &self,
        start_block_hash: &Blake2bHash,
        count: u32,
        include_body: bool,
        direction: Direction,
        txn_option: Option<&Transaction>,
    ) -> Result<Vec<Block>, BlockchainError> {
        self.chain_store
            .get_blocks(start_block_hash, count, include_body, direction, txn_option)
    }

    pub fn get_chain_info(
        &self,
        hash: &Blake2bHash,
        include_body: bool,
        txn_option: Option<&Transaction>,
    ) -> Result<ChainInfo, BlockchainError> {
        self.chain_store
            .get_chain_info(hash, include_body, txn_option)
    }

    pub fn get_slot_owner_at(
        &self,
        block_number: u32,
        offset: u32,
        txn_option: Option<&Transaction>,
    ) -> Result<(Validator, u16), BlockchainError> {
        let vrf_entropy = self
            .get_block_at(block_number - 1, false, txn_option)?
            .seed()
            .entropy();
        self.get_proposer_at(block_number, offset, vrf_entropy, txn_option)
            .map(|slot| (slot.validator, slot.number))
    }

    pub fn get_macro_blocks(
        &self,
        start_block_hash: &Blake2bHash,
        count: u32,
        include_body: bool,
        direction: Direction,
        election_blocks_only: bool,
        txn_option: Option<&Transaction>,
    ) -> Result<Vec<Block>, BlockchainError> {
        self.chain_store.get_macro_blocks(
            start_block_hash,
            count,
            include_body,
            direction,
            election_blocks_only,
            txn_option,
        )
    }

    /// Returns the current staking contract.
    pub fn get_staking_contract(&self) -> StakingContract {
        self.get_staking_contract_if_complete()
            .expect("We should always have the staking contract.")
    }

    /// Returns the current staking contract.
    pub fn get_staking_contract_if_complete(&self) -> Option<StakingContract> {
        let staking_contract_address = StakingContract::get_key_staking_contract();

        match self.state.accounts.get(&staking_contract_address, None) {
            Ok(Some(Account::Staking(x))) => Some(x),
            Err(_) => None,
            _ => {
                unreachable!()
            }
        }
    }

    /// Returns the number of accounts in the Accounts Tree. An account id defined as any leaf node
    /// in the tree.
    pub fn get_number_accounts(&self) -> u64 {
        self.state.accounts.size()
    }

    pub fn get_account(&self, address: &Address) -> Option<Account> {
        // TODO: Find a better place for this differentiation, it should be in a more general location.
        let key = if *address == Policy::STAKING_CONTRACT_ADDRESS {
            StakingContract::get_key_staking_contract()
        } else {
            KeyNibbles::from(address)
        };

        self.state
            .accounts
            .get(&key, None)
            .expect("Incomplete trie.")
    }

    /// Checks if we have seen some transaction with this hash inside the a validity window.
    pub fn tx_in_validity_window(
        &self,
        tx_hash: &Blake2bHash,
        validity_window_start: u32,
        txn_opt: Option<&Transaction>,
    ) -> bool {
        // Get a vector with all transactions corresponding to the given hash.
        let ext_hash_vec = self.history_store.get_ext_tx_by_hash(tx_hash, txn_opt);

        // If the vector is empty then we have never seen a transaction with this hash.
        if ext_hash_vec.is_empty() {
            return false;
        }

        for ext_tx in ext_hash_vec {
            // If the transaction is inside the validity window, return true.
            if ext_tx.block_number >= validity_window_start {
                return true;
            }
        }

        // If we didn't see any transaction inside the validity window then we can return false.
        false
    }

    /// Checks if we have seen some transaction with this hash inside the validity window. This is
    /// used to prevent replay attacks.
    pub fn contains_tx_in_validity_window(
        &self,
        tx_hash: &Blake2bHash,
        txn_opt: Option<&Transaction>,
    ) -> bool {
        let max_block_number = self
            .block_number()
            .saturating_sub(Policy::TRANSACTION_VALIDITY_WINDOW);
        self.tx_in_validity_window(tx_hash, max_block_number, txn_opt)
    }

    pub fn staking_contract_address(&self) -> Address {
        Policy::STAKING_CONTRACT_ADDRESS
    }

    #[cfg(feature = "metrics")]
    pub fn metrics(&self) -> Arc<BlockchainMetrics> {
        self.metrics.clone()
    }

    /// Retrieves the missing range of the accounts trie when it's incomplete.
    /// This function returns `None` when the trie is complete.
    pub fn get_missing_accounts_range(
        &self,
        txn_opt: Option<&Transaction>,
    ) -> Option<RangeFrom<KeyNibbles>> {
        let read_txn: ReadTransaction;
        let txn = match txn_opt {
            Some(txn) => txn,
            None => {
                read_txn = self.read_transaction();
                &read_txn
            }
        };

        self.state().accounts.tree.get_missing_range(txn)
    }
}
