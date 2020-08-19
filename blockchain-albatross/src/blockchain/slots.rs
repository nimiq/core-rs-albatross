use std::convert::TryInto;

#[cfg(feature = "metrics")]
use blockchain_base::chain_metrics::BlockchainMetrics;
use collections::BitSet;
use database::{ReadTransaction, Transaction};
use primitives::policy;
use primitives::slot::{Slot, SlotIndex, Slots, ValidatorSlots};
use vrf::{Rng, VrfSeed, VrfUseCase};

use crate::Blockchain;

/// Functions to do with slots and validators.
impl Blockchain {
    pub fn get_slots_for_epoch(&self, epoch: u32) -> Option<Slots> {
        let state = self.state.read();
        let current_epoch = policy::epoch_at(state.main_chain.head.block_number());

        let slots = if epoch == current_epoch {
            state.current_slots.as_ref()?.clone()
        } else if epoch == current_epoch - 1 {
            state.previous_slots.as_ref()?.clone()
        } else {
            let macro_block = self
                .chain_store
                .get_block_at(policy::election_block_of(epoch), true, None)?
                .unwrap_macro();
            macro_block.try_into().unwrap()
        };

        Some(slots)
    }

    pub fn get_validators_for_epoch(&self, epoch: u32) -> Option<ValidatorSlots> {
        if let Some(slots) = self.get_slots_for_epoch(epoch) {
            Some(slots.into())
        } else {
            None
        }
    }

    pub fn next_slots(&self, seed: &VrfSeed) -> Slots {
        self.get_staking_contract().select_validators(seed)
    }

    pub fn next_validators(&self, seed: &VrfSeed) -> ValidatorSlots {
        self.next_slots(seed).into()
    }

    pub fn get_slot_owner_at(
        &self,
        block_number: u32,
        view_number: u32,
        txn_option: Option<&Transaction>,
    ) -> (Slot, u16) {
        let state = self.state.read_recursive();

        let read_txn;
        let txn = if let Some(txn) = txn_option {
            txn
        } else {
            read_txn = ReadTransaction::new(&self.env);
            &read_txn
        };

        // Gets slots collection from either the cached ones, or from the election block.
        // Note: We need to handle the case where `state.block_number()` is at an election block
        // (so `state.current_slots` was already updated by it, pushing this epoch's slots to
        // `state.previous_slots` and deleting previous epoch's slots).
        let validator_slots_owned;
        let validator_slots = if policy::epoch_at(state.block_number())
            == policy::epoch_at(block_number)
            && !policy::is_election_block_at(state.block_number())
        {
            state.current_slots.as_ref().unwrap_or_else(|| {
                panic!(
                    "Missing epoch's slots for block {}.{}",
                    block_number, view_number
                )
            })
        } else if (policy::epoch_at(state.block_number()) == policy::epoch_at(block_number)
            && policy::is_election_block_at(state.block_number()))
            || (policy::epoch_at(state.block_number()) == policy::epoch_at(block_number) + 1
                && !policy::is_election_block_at(state.block_number()))
        {
            state.previous_slots.as_ref().unwrap_or_else(|| {
                panic!(
                    "Missing previous epoch's slots for block {}.{}",
                    block_number, view_number
                )
            })
        } else {
            let macro_block = self
                .chain_store
                .get_block_at(
                    policy::election_block_before(block_number),
                    true,
                    Some(&txn),
                )
                .expect("Can't fetch block")
                .unwrap_macro();

            validator_slots_owned = macro_block.try_into().unwrap();

            &validator_slots_owned
        };

        let disabled_slots = self
            .chain_store
            .get_block_at(policy::macro_block_before(block_number), true, Some(&txn))
            .expect("Can't fetch block")
            .unwrap_macro()
            .body
            .unwrap()
            .disabled_set;

        let slot_number = self.get_slot_owner_number_at(block_number, view_number, disabled_slots, Some(&txn));

        let slot = validator_slots
            .get(SlotIndex::Slot(slot_number))
            .unwrap_or_else(|| panic!("Expected slot {} to exist", slot_number));

        (slot, slot_number)
    }

    /// Calculate the slot owner number at a given block and view number.
    /// In combination with the active `Slots`, this can be used to retrieve the validator info.
    pub fn get_slot_owner_number_at(
        &self,
        block_number: u32,
        view_number: u32,
        disabled_slots: BitSet,
        txn_option: Option<&Transaction>,
    ) -> u16 {
        let seed = self
            .chain_store
            .get_block_at(block_number - 1, false, txn_option)
            .expect("Can't fetch block")
            .seed()
            .clone();

        // RNG for slot selection
        let mut rng = seed.rng(VrfUseCase::SlotSelection, view_number);

        // Check if all slots are disabled. In this case, we will accept any slot, since we want the
        // chain to progress.
        let all_disabled = disabled_slots.len() == policy::SLOTS as usize;

        // Sample until we find a slot that is not slashed
        loop {
            let slot_number = rng.next_u64_max(policy::SLOTS as u64) as u16;

            if !disabled_slots.contains(slot_number as usize) || all_disabled {
                return slot_number;
            }
        }
    }

    pub fn get_slot_owner_for_next_block(
        &self,
        view_number: u32,
        txn_option: Option<&Transaction>,
    ) -> (Slot, u16) {
        let block_number = self.block_number() + 1;

        self.get_slot_owner_at(block_number, view_number, txn_option)
    }
}
