use accounts::Accounts;
use block::MacroBlock;
#[cfg(feature = "metrics")]
use blockchain_base::chain_metrics::BlockchainMetrics;
use collections::bitset::BitSet;
use hash::Blake2bHash;
use primitives::policy;
use primitives::slot::{Slots, ValidatorSlots};

use crate::chain_info::ChainInfo;
use crate::reward_registry::{SlashRegistry, SlashedSetSelector};
use crate::transaction_cache::TransactionCache;

pub struct BlockchainState {
    pub accounts: Accounts,
    pub transaction_cache: TransactionCache,
    pub reward_registry: SlashRegistry,

    pub main_chain: ChainInfo,
    pub head_hash: Blake2bHash,

    pub macro_head: MacroBlock,
    pub macro_head_hash: Blake2bHash,

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

    /// This includes fork proof slashes and view changes.
    pub fn current_slashed_set(&self) -> BitSet {
        self.reward_registry.slashed_set(
            policy::epoch_at(self.block_number()),
            SlashedSetSelector::All,
            None,
        )
    }

    /// This includes fork proof slashes and view changes.
    pub fn last_slashed_set(&self) -> BitSet {
        self.reward_registry.slashed_set(
            policy::epoch_at(self.block_number()) - 1,
            SlashedSetSelector::All,
            None,
        )
    }

    pub fn reward_registry(&self) -> &SlashRegistry {
        &self.reward_registry
    }
}
