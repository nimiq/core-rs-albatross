use nimiq_primitives::policy;
use nimiq_primitives::slots::Validators;
use nimiq_vrf::VrfSeed;

use crate::Blockchain;
use nimiq_account::StakingContract;

/// Implements methods to handle slots and validators.
impl Blockchain {
    /// Gets the validators for a given epoch.
    pub fn get_validators_for_epoch(&self, epoch: u32) -> Option<Validators> {
        let current_epoch = policy::epoch_at(self.state.main_chain.head.block_number());

        let slots = if epoch == current_epoch {
            self.state.current_slots.as_ref()?.clone()
        } else if epoch == current_epoch - 1 {
            self.state.previous_slots.as_ref()?.clone()
        } else {
            let macro_block = self
                .chain_store
                .get_block_at(policy::election_block_of(epoch), true, None)?
                .unwrap_macro();
            macro_block.get_validators().unwrap()
        };

        Some(slots)
    }

    /// Calculates the next validators from a given seed.
    pub fn next_validators(&self, seed: &VrfSeed) -> Validators {
        StakingContract::select_validators(
            &self.state().accounts.tree,
            &self.read_transaction(),
            seed,
        )
    }
}
