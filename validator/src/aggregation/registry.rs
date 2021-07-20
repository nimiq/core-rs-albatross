use std::collections::HashSet;

use bls::PublicKey;
use collections::BitSet;
use handel::identity::{Identity, IdentityRegistry, WeightRegistry};
use primitives::policy;
use primitives::slots::Validators;

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

    pub fn get_slots(&self, idx: u16) -> Vec<u16> {
        let validator = &self.validators.validators[idx as usize];

        (validator.slot_range.0..validator.slot_range.1).collect()
    }
}

impl IdentityRegistry for ValidatorRegistry {
    fn public_key(&self, slot_number: usize) -> Option<PublicKey> {
        self.validators
            // Get the validator for with id
            .get_validator(slot_number as u16)
            // Get the public key for this validator
            .public_key
            // and uncompress it
            .uncompress()
            .map(|c| *c) // necessary?
    }

    fn signers_identity(&self, slots: &BitSet) -> Identity {
        if slots.is_empty() {
            // if there is no signers there is no identity.
            Identity::None
        } else {
            // create a set of validator ids corresponding to the slots
            let mut ids: HashSet<u16> = HashSet::new();
            for slot in slots.iter() {
                // insert each validator_address if there is one.
                let _ = ids.insert(self.validators.get_band_from_slot(slot as u16));
            }

            // if there is exactly no signer it needs to be rejected.
            // should never happen as those get rejected earlier.
            if ids.is_empty() {
                Identity::None
            } else {
                // make sure that the bitset for the given validator's slots is exactly the same as the once given,
                // as otherwise there would be a 'partial' slot layout for at least one of the validators.
                // Since handel will not combine overlapping contributions those need to be rejected.
                // this holds for Single and Multiple signatories.
                let mut validators_slots = BitSet::new();
                for validator_id in ids.iter() {
                    for slot in self.get_slots(*validator_id).iter() {
                        validators_slots.insert(*slot as usize);
                    }
                }

                if &validators_slots != slots {
                    // reject any slots which are not exhaustive for their validators.
                    Identity::None
                } else if ids.len() == 1 {
                    Identity::Single(*ids.iter().next().unwrap() as usize)
                } else {
                    Identity::Multiple(ids.iter().map(|id| *id as usize).collect())
                }
            }
        }
    }
}

impl WeightRegistry for ValidatorRegistry {
    fn weight(&self, id: usize) -> Option<usize> {
        if (0..policy::SLOTS).contains(&(id as u16)) {
            Some(1)
        } else {
            None
        }
    }
}
