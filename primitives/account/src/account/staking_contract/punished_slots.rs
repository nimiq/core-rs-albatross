use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
};

use nimiq_collections::BitSet;
use nimiq_keys::Address;
use nimiq_primitives::{
    coin::Coin,
    policy::Policy,
    slots_allocation::{JailedValidator, PenalizedSlot},
};
use nimiq_serde::{Deserialize, Serialize};

/// Data structure to keep track of the punished slots of the previous and current batch.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PunishedSlots {
    // The validator slots that lost rewards (i.e. are not eligible to receive rewards) during
    // the current epoch.
    pub current_batch_punished_slots: BTreeMap<Address, BTreeSet<u16>>,
    // The validator slots that lost rewards (i.e. are not eligible to receive rewards) during
    // the previous batch.
    pub previous_batch_punished_slots: BitSet,
}

impl PunishedSlots {
    pub fn new(
        current_batch_punished_slots: BTreeMap<Address, BTreeSet<u16>>,
        previous_batch_punished_slots: BitSet,
    ) -> Self {
        Self {
            current_batch_punished_slots,
            previous_batch_punished_slots,
        }
    }

    /// Registers a new jail for a given validator.
    /// If the event occurred in the current batch, it will only affect the current set.
    /// If the event was only reported in a subsequent batch, it will affect both sets,
    /// except if the validator was not elected in the corresponding batches.
    pub fn register_jail(
        &mut self,
        jailed_validator: &JailedValidator,
        reporting_block: u32,
        new_epoch_slot_range: Option<Range<u16>>,
    ) -> (BitSet, Option<BTreeSet<u16>>) {
        let old_previous_batch_punished_slots = self.previous_batch_punished_slots.clone();
        let old_current_batch_punished_slots = self
            .current_batch_punished_slots
            .get(&jailed_validator.validator_address)
            .cloned();

        // If the event happened in the same epoch, the slot range will be determined by the event.
        // If the event happened in a previous epoch, we get the current slot range as an argument.
        let current_batch_slot_range = if Policy::epoch_at(jailed_validator.offense_event_block)
            == Policy::epoch_at(reporting_block)
        {
            Some(jailed_validator.slots.clone())
        } else {
            new_epoch_slot_range.clone()
        };

        // We always add the validator to the current set.
        // If the validator is wasn't elected for the epoch, then we don't punish.
        if let Some(slots) = current_batch_slot_range {
            let entry = self
                .current_batch_punished_slots
                .entry(jailed_validator.validator_address.clone())
                .or_default();

            for slot in slots {
                entry.insert(slot);
            }
        }

        // If the event happened in an earlier batch, we also punish
        // the slots in the previous batch set.
        if Policy::batch_at(jailed_validator.offense_event_block)
            < Policy::batch_at(reporting_block)
        {
            // The following code assumes that the reporting window cannot span more than 2 epochs.
            assert!(
                Policy::epoch_at(reporting_block)
                    - Policy::epoch_at(jailed_validator.offense_event_block)
                    <= 1,
                "Cannot handle events that happened more than one epoch ago"
            );

            // If the event happened in the same epoch as the previous batch, the slot range will be determined by the event.
            // If the event happened in a different epoch than the previous batch, we get the current slot range as an argument.
            let previous_batch_slot_range =
                if Policy::epoch_at(jailed_validator.offense_event_block)
                    == Policy::epoch_at(reporting_block - Policy::blocks_per_batch())
                {
                    Some(jailed_validator.slots.clone())
                } else {
                    new_epoch_slot_range
                };

            // If the validator is wasn't elected for the corresponding epoch, then we don't punish.
            if let Some(slots) = previous_batch_slot_range {
                for slot in slots {
                    self.previous_batch_punished_slots.insert(slot as usize);
                }
            }
        }

        (
            old_previous_batch_punished_slots,
            old_current_batch_punished_slots,
        )
    }

    /// Reverts a new penalty for a given slot.
    pub fn revert_register_jail(
        &mut self,
        jailed_validator: &JailedValidator,
        old_previous_batch_punished_slots: BitSet,
        old_current_batch_punished_slots: Option<BTreeSet<u16>>,
    ) {
        self.previous_batch_punished_slots = old_previous_batch_punished_slots;
        if let Some(set) = old_current_batch_punished_slots {
            assert!(
                self.current_batch_punished_slots
                    .insert(jailed_validator.validator_address.clone(), set)
                    .is_some(),
                "Missing jailed validator"
            );
        } else {
            assert!(
                self.current_batch_punished_slots
                    .remove(&jailed_validator.validator_address)
                    .is_some(),
                "Missing jailed validator"
            );
        }
    }

    /// Registers a new penalty for a given slot.
    /// The penalty always affects the batch in which the event happened.
    /// If the event was only reported in the subsequent batch, but in the same epoch,
    /// it will affect the current batch too.
    pub fn register_penalty(
        &mut self,
        penalized_slot: &PenalizedSlot,
        reporting_block: u32,
    ) -> (bool, bool) {
        let mut newly_punished_previous_batch = false;
        let mut newly_punished_current_batch = false;

        // Reported during subsequent batch, so changes the previous set.
        if Policy::batch_at(penalized_slot.offense_event_block) + 1
            == Policy::batch_at(reporting_block)
        {
            newly_punished_previous_batch = !self
                .previous_batch_punished_slots
                .contains(penalized_slot.slot as usize);
            self.previous_batch_punished_slots
                .insert(penalized_slot.slot as usize);
        }
        // Only apply the penalty to the current set if the epoch is the same.
        // On a new epoch the validator may not be elected. Even if it is,
        // there is no straightforward mapping between the slots of the previous and new epoch.
        if Policy::epoch_at(penalized_slot.offense_event_block) == Policy::epoch_at(reporting_block)
        {
            newly_punished_current_batch = self
                .current_batch_punished_slots
                .entry(penalized_slot.validator_address.clone())
                .or_default()
                .insert(penalized_slot.slot);
        }

        (newly_punished_previous_batch, newly_punished_current_batch)
    }

    /// Reverts a new penalty for a given slot.
    pub fn revert_register_penalty(
        &mut self,
        penalized_slot: &PenalizedSlot,
        newly_punished_previous_batch: bool,
        newly_punished_current_batch: bool,
    ) {
        if newly_punished_previous_batch {
            self.previous_batch_punished_slots
                .remove(penalized_slot.slot as usize);
        }

        if newly_punished_current_batch {
            let entry = self
                .current_batch_punished_slots
                .get_mut(&penalized_slot.validator_address)
                .expect("Missing validator");

            assert!(
                entry.remove(&penalized_slot.slot),
                "Should have penalized slot"
            );

            if entry.is_empty() {
                self.current_batch_punished_slots
                    .remove(&penalized_slot.validator_address);
            }
        }
    }

    /// At the end of a batch, we update the previous bitset and remove reactivated validators from the current bitset.
    /// However, at the end of an epoch the current bitset is reset.
    /// This function should be called at every macro block.
    pub fn finalize_batch(
        &mut self,
        block_number: u32,
        current_active_validators: &BTreeMap<Address, Coin>,
    ) {
        // Updates the previous bitset with the current one.
        self.previous_batch_punished_slots = self.current_batch_punished_slots();

        if Policy::is_election_block_at(block_number) {
            // At an epoch boundary, the next starting set is empty.
            self.current_batch_punished_slots = Default::default();
        } else {
            // Remove all validators that are active again.
            assert!(Policy::is_macro_block_at(block_number));

            self.current_batch_punished_slots
                .retain(|validator_address, _| {
                    current_active_validators.get(validator_address).is_none()
                });
        }
    }

    /// Returns the bitset representing which validator slots will be prohibited from producing micro blocks or
    /// proposing macro blocks in the next batch.
    /// It is exercising the `finalize_batch` transition.
    /// This method should only be needed to populate the corresponding field in *macro blocks*.
    pub fn next_batch_initial_punished_set(
        &self,
        block_number: u32,
        current_active_validators: &BTreeMap<Address, Coin>,
    ) -> BitSet {
        let mut punished_slots = self.clone();
        punished_slots.finalize_batch(block_number, current_active_validators);

        punished_slots.current_batch_punished_slots()
    }

    /// Returns a BitSet of slots that were punished in the current epoch.
    pub fn current_batch_punished_slots(&self) -> BitSet {
        let mut bitset = BitSet::new();
        for slots in self.current_batch_punished_slots.values() {
            for &slot in slots {
                bitset.insert(slot as usize);
            }
        }
        bitset
    }

    /// Returns a BitSet of slots that were punished in the current epoch.
    pub fn current_batch_punished_slots_map(&self) -> &BTreeMap<Address, BTreeSet<u16>> {
        &self.current_batch_punished_slots
    }

    /// Returns a BitSet of slots that were punished in the previous epoch.
    pub fn previous_batch_punished_slots(&self) -> &BitSet {
        &self.previous_batch_punished_slots
    }
}
