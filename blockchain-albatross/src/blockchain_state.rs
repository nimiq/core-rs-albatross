#[cfg(feature = "metrics")]
use crate::chain_metrics::BlockchainMetrics;
use accounts::Accounts;
use block::MacroBlock;
use hash::Blake2bHash;
use primitives::slot::{Slots, ValidatorSlots};

use crate::chain_info::ChainInfo;
use crate::transaction_cache::TransactionCache;

/// A struct that keeps the current state of the blockchain. It summarizes the information known to
/// a validator at the head of the blockchain.
pub struct BlockchainState {
    // The accounts tree.
    pub accounts: Accounts,
    // The cache of transactions.
    pub transaction_cache: TransactionCache,
    // The chain info for the head of the main chain.
    pub main_chain: ChainInfo,
    // The hash of the head of the main chain.
    pub head_hash: Blake2bHash,
    // The chain info for the last macro block.
    pub macro_info: ChainInfo,
    // The hash of the last macro block.
    pub macro_head_hash: Blake2bHash,
    // The last election macro block.
    pub election_head: MacroBlock,
    // The hash of the last election macro block.
    pub election_head_hash: Blake2bHash,
    // The validator slots for the current epoch.
    pub current_slots: Option<Slots>,
    // The validator slots for the previous epoch.
    pub previous_slots: Option<Slots>,
}

impl BlockchainState {
    pub fn accounts(&self) -> &Accounts {
        &self.accounts
    }

    pub fn transaction_cache(&self) -> &TransactionCache {
        &self.transaction_cache
    }

    pub fn block_number(&self) -> u32 {
        self.main_chain.head.block_number()
    }

    pub fn current_slots(&self) -> Option<&Slots> {
        self.current_slots.as_ref()
    }

    pub fn last_slots(&self) -> Option<&Slots> {
        self.previous_slots.as_ref()
    }

    pub fn current_validators(&self) -> Option<&ValidatorSlots> {
        Some(&self.current_slots.as_ref()?.validator_slots)
    }

    pub fn last_validators(&self) -> Option<&ValidatorSlots> {
        Some(&self.previous_slots.as_ref()?.validator_slots)
    }

    pub fn main_chain(&self) -> &ChainInfo {
        &self.main_chain
    }
}
