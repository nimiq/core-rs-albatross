use std::collections::HashSet;

use bls::PublicKey;
use collections::BitSet;
use handel::identity::{Identity, IdentityRegistry, WeightRegistry};
use primitives::policy;
use primitives::slot::{SlotCollection, ValidatorSlots};

/// Implementation for handel registry using a `Validators` list.
#[derive(Debug)]
pub(crate) struct ValidatorRegistry {
    validators: ValidatorSlots,
}

impl ValidatorRegistry {
    pub fn new(validators: ValidatorSlots) -> Self {
        Self { validators }
    }

    pub fn len(&self) -> usize {
        self.validators.len()
    }

    pub fn get_slots(&self, idx: u16) -> Vec<u16> {
        self.validators.get_slots(idx)
    }
}

impl IdentityRegistry for ValidatorRegistry {
    fn public_key(&self, id: usize) -> Option<PublicKey> {
        self.validators
            // Get the band for the validator with id
            .get_by_slot_number(id as u16)
            .and_then(|slot_band| {
                slot_band
                    // Get the public key for this band
                    .public_key()
                    // and uncompress it
                    .uncompress()
                    .map(|c| *c) // necessary?
            })
    }

    fn signers_identity(&self, slots: &BitSet) -> Identity {
        if slots.len() == 0 {
            // if there is no signers there is no identity.
            Identity::None
        } else {
            // create a set of validator ids corresponding to the slots
            let mut ids: HashSet<u16> = HashSet::new();
            for slot in slots.iter() {
                // insert each validator_id if there is one.
                if let Some(id) = self.validators.get_band_number_by_slot_number(slot as u16) {
                    let _ = ids.insert(id);
                } else {
                    // If there is None this bitset is not valid and must be rejected
                    return Identity::None;
                }
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
                    for slot in self.validators.get_slots(*validator_id).iter() {
                        validators_slots.insert(*slot as usize);
                    }
                }

                if &validators_slots != slots {
                    // reject any slots which are not exhaustive for their validators.
                    Identity::None
                } else {
                    if ids.len() == 1 {
                        Identity::Single(*ids.iter().next().unwrap() as usize)
                    } else {
                        Identity::Multiple(ids.iter().map(|id| *id as usize).collect())
                    }
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
