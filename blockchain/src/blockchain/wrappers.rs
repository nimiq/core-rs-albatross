use std::ops::RangeFrom;
#[cfg(feature = "metrics")]
use std::sync::Arc;

use nimiq_account::{Account, BlockState, DataStore, ReservedBalance, StakingContract};
use nimiq_block::Block;
use nimiq_blockchain_interface::{AbstractBlockchain, BlockchainError, ChainInfo, Direction};
use nimiq_database::TransactionProxy as DBTransaction;
use nimiq_hash::Blake2bHash;
use nimiq_keys::Address;
use nimiq_primitives::{
    account::AccountError, key_nibbles::KeyNibbles, policy::Policy, slots_allocation::Slot,
};
use nimiq_transaction::Transaction;

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
        txn_option: Option<&DBTransaction>,
    ) -> Result<Block, BlockchainError> {
        self.chain_store
            .get_block_at(height, include_body, txn_option)
    }

    pub fn get_block(
        &self,
        hash: &Blake2bHash,
        include_body: bool,
        txn_option: Option<&DBTransaction>,
    ) -> Result<Block, BlockchainError> {
        self.chain_store.get_block(hash, include_body, txn_option)
    }

    pub fn get_blocks(
        &self,
        start_block_hash: &Blake2bHash,
        count: u32,
        include_body: bool,
        direction: Direction,
        txn_option: Option<&DBTransaction>,
    ) -> Result<Vec<Block>, BlockchainError> {
        self.chain_store
            .get_blocks(start_block_hash, count, include_body, direction, txn_option)
    }

    pub fn get_chain_info(
        &self,
        hash: &Blake2bHash,
        include_body: bool,
        txn_option: Option<&DBTransaction>,
    ) -> Result<ChainInfo, BlockchainError> {
        self.chain_store
            .get_chain_info(hash, include_body, txn_option)
    }

    /// Returns information about the proposer at the given block height and offset.
    /// The offset is the block number for micro blocks + skip blocks and the round number for macro blocks.
    pub fn get_proposer_at(
        &self,
        block_number: u32,
        offset: u32,
        txn_option: Option<&DBTransaction>,
    ) -> Result<Slot, BlockchainError> {
        let vrf_entropy = self
            .get_block_at(block_number - 1, false, txn_option)?
            .seed()
            .entropy();

        self.get_proposer(block_number, offset, vrf_entropy, txn_option)
    }

    /// Returns information about the proposer of the block with the given `block_hash`.
    pub fn get_proposer_of(
        &self,
        block_hash: &Blake2bHash,
        txn_option: Option<&DBTransaction>,
    ) -> Result<Slot, BlockchainError> {
        let block = self.get_block(block_hash, false, txn_option)?;

        let offset = block
            .vrf_offset()
            .ok_or(BlockchainError::BlockJustificationNotFound)?;

        let vrf_entropy = self
            .get_block(block.parent_hash(), false, txn_option)?
            .seed()
            .entropy();

        self.get_proposer(block.block_number(), offset, vrf_entropy, txn_option)
    }

    pub fn get_macro_blocks(
        &self,
        start_block_hash: &Blake2bHash,
        count: u32,
        include_body: bool,
        direction: Direction,
        election_blocks_only: bool,
        txn_option: Option<&DBTransaction>,
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
        self.get_staking_contract_if_complete(None)
            .expect("We should always have the staking contract.")
    }

    /// Returns the current staking contract.
    pub fn get_staking_contract_if_complete(
        &self,
        txn_option: Option<&DBTransaction>,
    ) -> Option<StakingContract> {
        let staking_contract = self
            .state
            .accounts
            .get(&Policy::STAKING_CONTRACT_ADDRESS, txn_option)
            .ok()?;
        match staking_contract {
            Account::Staking(x) => Some(x),
            _ => unreachable!(),
        }
    }

    /// Returns the contract data store for the staking contract.
    pub fn get_staking_contract_store(&self) -> DataStore {
        self.state
            .accounts
            .data_store(&Policy::STAKING_CONTRACT_ADDRESS)
    }

    /// Returns the number of accounts in the Accounts Tree. An account id defined as any leaf node
    /// in the tree.
    pub fn get_number_accounts(&self) -> u64 {
        self.state.accounts.size()
    }

    pub fn get_account_if_complete(&self, address: &Address) -> Option<Account> {
        if let Ok(account) = self.state.accounts.get(address, None) {
            Some(account)
        } else {
            warn!(%address, "Could not get account for address");
            None
        }
    }

    pub fn reserve_balance(
        &self,
        account: &Account,
        transaction: &Transaction,
        reserved_balance: &mut ReservedBalance,
    ) -> Result<(), AccountError> {
        let block_state = BlockState::new(self.block_number(), self.timestamp());
        self.state.accounts.reserve_balance(
            account,
            transaction,
            reserved_balance,
            &block_state,
            None,
        )
    }

    pub fn release_balance(
        &self,
        account: &Account,
        transaction: &Transaction,
        reserved_balance: &mut ReservedBalance,
    ) -> Result<(), AccountError> {
        self.state
            .accounts
            .release_balance(account, transaction, reserved_balance, None)
    }

    /// Checks if we have seen some transaction with this hash inside the a validity window.
    pub fn tx_in_validity_window(
        &self,
        tx_hash: &Blake2bHash,
        validity_window_start: u32,
        txn_opt: Option<&DBTransaction>,
    ) -> bool {
        // Get a vector with all transactions corresponding to the given hash.
        let ext_hash_vec = self.history_store.get_hist_tx_by_hash(tx_hash, txn_opt);

        // If the vector is empty then we have never seen a transaction with this hash.
        if ext_hash_vec.is_empty() {
            return false;
        }

        for hist_tx in ext_hash_vec {
            // If the transaction is inside the validity window, return true.
            if hist_tx.block_number >= validity_window_start {
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
        txn_opt: Option<&DBTransaction>,
    ) -> bool {
        let max_block_number = self
            .block_number()
            .saturating_sub(Policy::transaction_validity_window_blocks());
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
        txn_opt: Option<&DBTransaction>,
    ) -> Option<RangeFrom<KeyNibbles>> {
        let read_txn: DBTransaction;
        let txn = match txn_opt {
            Some(txn) => txn,
            None => {
                read_txn = self.read_transaction();
                &read_txn
            }
        };

        self.state().accounts.tree.get_missing_range(txn)
    }

    /// Check if all blockchain conditions to run a validator are met.
    /// These are:
    /// - can verify/extend history tree
    /// - can enforce transaction validity window
    /// The second subsumes the first.
    /// All conditions are checked against the next block (which potentially hasn't been produced yet).
    pub fn can_enforce_validity_window(&self) -> bool {
        if self.state.can_verify_history {
            // If we are at the genesis block or in the first epoch and we can verify the history,
            // we can also enforce the validity window.
            if Policy::epoch_at(self.block_number() + 1) <= 1 {
                return true;
            }

            // Since the transaction validity window is smaller than an epoch and we're in epoch 2 or greater,
            // the subtraction cannot fail.
            // We then calculate the election block greater or equal to the first validity window block
            // to be used in `length_at`. Using micro blocks in `length_at` can be tricky as they might be empty
            // and not occur in the history store at all. Using the election block of the epoch will guarantee
            // to return the length at the most recent existing block in this epoch.
            let first_validity_window_block =
                self.block_number() + 1 - Policy::transaction_validity_window_blocks();
            let election_block = Policy::election_block_after(first_validity_window_block - 1);

            // Check whether we do have a history tree for the epoch of the first validity window block.
            // Length at will use the election block or the most recent existing block (in the same epoch).
            return self.history_store.length_at(election_block, None) > 0;
        }
        false
    }
}
