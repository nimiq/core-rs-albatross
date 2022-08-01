use nimiq_account::StakingContract;
use nimiq_collections::BitSet;
use nimiq_database::Transaction;
use nimiq_primitives::policy;
use nimiq_primitives::slots::{Validator, Validators};
use nimiq_vrf::{Rng, VrfEntropy, VrfSeed, VrfUseCase};

use crate::{AbstractBlockchain, Blockchain};

pub struct Slot {
    pub number: u16,
    pub band: u16,
    pub validator: Validator,
}

/// Implements methods to handle slots and validators.
impl Blockchain {
    /// Gets the active validators for a given epoch.
    pub fn get_validators_for_epoch(
        &self,
        epoch: u32,
        txn: Option<&Transaction>,
    ) -> Option<Validators> {
        let current_epoch = policy::epoch_at(self.state.main_chain.head.block_number());

        if epoch == current_epoch {
            self.state.current_slots.clone()
        } else if epoch + 1 == current_epoch {
            self.state.previous_slots.clone()
        } else if epoch == 0 {
            None
        } else {
            self.chain_store
                .get_block_at(policy::election_block_of(epoch - 1), true, txn)?
                .unwrap_macro()
                .get_validators()
        }
    }

    /// Calculates the next validators from a given seed.
    pub fn next_validators(&self, seed: &VrfSeed) -> Validators {
        StakingContract::select_validators(
            &self.state().accounts.tree,
            &self.read_transaction(),
            seed,
        )
    }

    pub fn get_proposer_at(
        &self,
        block_number: u32,
        offset: u32,
        vrf_entropy: VrfEntropy,
        txn: Option<&Transaction>,
    ) -> Option<Slot> {
        // Fetch the latest macro block that precedes the block at the given block_number.
        // We use the disabled_slots set from that macro block for the slot selection.
        let macro_block = self.get_block_at(policy::macro_block_before(block_number), true, txn)?;
        let disabled_slots = macro_block.unwrap_macro().body.unwrap().disabled_set;

        // Compute the slot number of the next proposer.
        let slot_number = Self::compute_slot_number(offset, vrf_entropy, disabled_slots);

        // Fetch the validators that are active in given block's epoch.
        let epoch_number = policy::epoch_at(block_number);
        let validators = self.get_validators_for_epoch(epoch_number, txn)?;

        // Get the validator that owns the proposer slot.
        let validator = validators.get_validator_by_slot_number(slot_number);

        // Also get the slot band for convenient access.
        let slot_band = validators.get_band_from_slot(slot_number);

        Some(Slot {
            number: slot_number,
            band: slot_band,
            validator: validator.clone(),
        })
    }

    fn compute_slot_number(offset: u32, vrf_entropy: VrfEntropy, disabled_slots: BitSet) -> u16 {
        // RNG for slot selection
        let mut rng = vrf_entropy.rng(VrfUseCase::ViewSlotSelection);

        // Create a list of viable slots.
        let mut slots: Vec<u16> = if disabled_slots.len() == policy::SLOTS as usize {
            // If all slots are disabled, we will accept any slot, since we want the
            // chain to progress.
            (0..policy::SLOTS).collect()
        } else {
            // Otherwise, we will only accept slots that are not disabled.
            (0..policy::SLOTS)
                .filter(|slot| !disabled_slots.contains(*slot as usize))
                .collect()
        };

        // Shuffle the slots vector using the Fisher–Yates shuffle.
        for i in (1..slots.len()).rev() {
            let r = rng.next_u64_max((i + 1) as u64) as usize;
            slots.swap(r, i);
        }

        // Now simply take the offset modulo the number of viable slots and that will give us
        // the chosen slot.
        slots[offset as usize % slots.len()]
    }
}
