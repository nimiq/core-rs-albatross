use std::ops::RangeFrom;
#[cfg(feature = "metrics")]
use std::sync::Arc;

use nimiq_account::{Account, BlockState, DataStore, ReservedBalance, StakingContract};
use nimiq_block::Block;
use nimiq_blockchain_interface::{AbstractBlockchain, BlockchainError, ChainInfo, Direction};
use nimiq_database::{mdbx::MdbxReadTransaction as DBTransaction, traits::WriteTransaction};
use nimiq_hash::Blake2bHash;
use nimiq_keys::Address;
use nimiq_primitives::{
    account::{AccountError, AccountType},
    coin::Coin,
    key_nibbles::KeyNibbles,
    policy::Policy,
    slots_allocation::Slot,
};
use nimiq_transaction::{historic_transaction::RawTransactionHash, Transaction};

#[cfg(feature = "metrics")]
use crate::chain_metrics::BlockchainMetrics;
use crate::{blockchain_state::BlockchainState, interface::HistoryInterface, Blockchain};

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
        let predecessor = block_number
            .checked_sub(1)
            .ok_or(BlockchainError::BlockNotFound(block_number))?;
        let vrf_entropy = self
            .get_block_at(predecessor, false, txn_option)?
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

        let vrf_entropy = self
            .get_block(block.parent_hash(), false, txn_option)?
            .seed()
            .entropy();

        self.get_proposer(
            block.block_number(),
            block.vrf_offset(),
            vrf_entropy,
            txn_option,
        )
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
    pub fn get_staking_contract_store(&self) -> DataStore<'_> {
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
        self.state.accounts.get(address, None).ok()
    }

    /// The given account must correspond to the sender of the given transaction.
    pub fn reserve_balance(
        &self,
        account: &Account,
        transaction: &Transaction,
        reserved_balance: &mut ReservedBalance,
    ) -> Result<(), AccountError> {
        let block_state =
            BlockState::new(self.block_number(), self.timestamp(), self.head().version());
        self.state.accounts.reserve_balance(
            account,
            transaction,
            reserved_balance,
            &block_state,
            None,
        )
    }

    /// The given account must correspond to the sender of the given transaction.
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

    /// For a bridge-sender transaction with a non-zero fee, reserve the fee against the
    /// burn-proof signer's account. A permissionless bridge burn-release pays its fee
    /// from the signer at commit (the bridge itself reserves/deducts only `value`), so
    /// the fee must be reserved against the signer here or the transaction could pass
    /// mempool admission and then fail at commit. No-op for non-bridge or zero-fee txs.
    ///
    /// The fee is reserved within the bridge sender's `ReservedBalance` (via `reserve_for`),
    /// which aggregates multiple burn-releases routed through the same bridge by the same
    /// signer. NOTE (follow-up): it does not aggregate a signer's obligations across
    /// different bridges or with the signer's own (non-bridge) transactions; those
    /// topologies can still over-commit a signer and need signer-bucket tracking.
    pub fn reserve_bridge_signer_fee(
        &self,
        transaction: &Transaction,
        reserved_balance: &mut ReservedBalance,
    ) -> Result<(), AccountError> {
        if transaction.sender_type != AccountType::Bridge || transaction.fee == Coin::ZERO {
            return Ok(());
        }

        let signer = self.state.accounts.extract_signer_address(transaction)?;
        let signer_balance = self
            .get_account_if_complete(&signer)
            .map(|account| account.balance())
            .unwrap_or(Coin::ZERO);

        reserved_balance.reserve_for(&signer, signer_balance, transaction.fee)
    }

    /// Release a previously reserved bridge signer fee (inverse of
    /// [`Self::reserve_bridge_signer_fee`]). No-op for non-bridge or zero-fee txs.
    pub fn release_bridge_signer_fee(
        &self,
        transaction: &Transaction,
        reserved_balance: &mut ReservedBalance,
    ) {
        if transaction.sender_type != AccountType::Bridge || transaction.fee == Coin::ZERO {
            return;
        }

        if let Ok(signer) = self.state.accounts.extract_signer_address(transaction) {
            reserved_balance.release_for(&signer, transaction.fee);
        }
    }

    /// Checks if we have seen some transaction with this hash inside the validity window. This is
    /// used to prevent replay attacks.
    pub fn contains_tx_in_validity_window(
        &self,
        tx_hash: &RawTransactionHash,
        txn_opt: Option<&DBTransaction>,
    ) -> bool {
        self.history_store.tx_in_validity_window(tx_hash, txn_opt)
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

        self.state.accounts.tree.get_missing_range(txn)
    }

    /// Removes the history of a given epoch
    pub fn remove_epoch_history(&mut self, epoch_number: u32) {
        let mut txn = self.write_transaction();

        self.history_store.remove_history(&mut txn, epoch_number);

        txn.commit();
    }
}
