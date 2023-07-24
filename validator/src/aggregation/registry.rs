use std::{collections::HashSet, ops};

use nimiq_bls::PublicKey;
use nimiq_collections::BitSet;
use nimiq_handel::identity::{Identity, IdentityRegistry, WeightRegistry};
use nimiq_primitives::{policy::Policy, slots::Validators};

/// Implementation for Handel registry using a `Validators` list.
#[derive(Debug)]
pub(crate) struct ValidatorRegistry {
    validators: Validators,
}

impl ValidatorRegistry {
    pub fn new(validators: Validators) -> Self {
        Self { validators }
    }

    pub fn len(&self) -> usize {
        self.validators.num_validators()
    }

    pub fn get_slots(&self, idx: u16) -> ops::Range<u16> {
        self.validators.validators[idx as usize].slots.clone()
    }
}

impl IdentityRegistry for ValidatorRegistry {
    fn public_key(&self, slot_number: usize) -> Option<PublicKey> {
        self.validators
            // Get the validator for with id
            .get_validator_by_slot_number(slot_number as u16)
            // Get the public key for this validator
            .voting_key
            // and uncompress it
            .uncompress()
            .map(|c| *c) // necessary?
    }

    fn signers_identity(&self, slots: &BitSet) -> Identity {
        if slots.is_empty() {
            // if there is no signers there is no identity.
            return Identity::new(BitSet::default());
        }

        // Create a set of validator ids corresponding to the slots
        let mut ids: HashSet<u16> = HashSet::new();
        for slot in slots.iter() {
            // Insert each validator_address if there is one.
            let _ = ids.insert(self.validators.get_band_from_slot(slot as u16));
        }

        // If there is no signer it needs to be rejected.
        // Should never happen as those get rejected earlier.
        if ids.is_empty() {
            return Identity::new(BitSet::default());
        }

        // Make sure that the bitset for the given validator's slots is exactly the same as the once given,
        // as otherwise there would be a 'partial' slot layout for at least one of the validators.
        // Since handel will not combine overlapping contributions those need to be rejected.
        // This holds for Single and Multiple signatories.
        let mut validators_slots = BitSet::new();
        for validator_id in ids.iter() {
            for slot in self.get_slots(*validator_id) {
                validators_slots.insert(slot as usize);
            }
        }

        if &validators_slots != slots {
            // Reject any slots which are not exhaustive for their validators.
            return Identity::new(BitSet::default());
        }

        Identity::new(BitSet::from_iter(ids.iter().map(|id| *id as usize)))
    }
}

impl WeightRegistry for ValidatorRegistry {
    fn weight(&self, id: usize) -> Option<usize> {
        if (0..Policy::SLOTS).contains(&(id as u16)) {
            Some(1)
        } else {
            None
        }
    }
}
