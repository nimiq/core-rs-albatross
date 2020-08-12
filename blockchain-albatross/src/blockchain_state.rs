use accounts::Accounts;
use block::MacroBlock;
#[cfg(feature = "metrics")]
use blockchain_base::chain_metrics::BlockchainMetrics;
use hash::Blake2bHash;
use primitives::slot::{Slots, ValidatorSlots};

use crate::chain_info::ChainInfo;
use crate::transaction_cache::TransactionCache;

pub struct BlockchainState {
    pub accounts: Accounts,
    pub transaction_cache: TransactionCache,

    pub main_chain: ChainInfo,
    pub head_hash: Blake2bHash,

    pub macro_info: ChainInfo,
    pub macro_head_hash: Blake2bHash,

    pub election_head: MacroBlock,
    pub election_head_hash: Blake2bHash,

    // TODO: Instead of Option, we could use a Cell here and use replace on it. That way we know
    //       at compile-time that there is always a valid value in there.
    pub current_slots: Option<Slots>,
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
