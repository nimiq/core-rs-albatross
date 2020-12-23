use bls::PublicKey;
use handel::identity::{IdentityRegistry, WeightRegistry};
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
}

impl WeightRegistry for ValidatorRegistry {
    fn weight(&self, id: usize) -> Option<usize> {
        if (0..policy::SLOTS).contains(&(id as u16)) {
            Some(1)
        } else {
            None
        }
        // self.validators
        //     // Get the validator band for the id
        //     .get_by_band_number(id as u16)
        //     // Retrieve number of slots for this band
        //     .and_then(|slot_band| Some(slot_band.num_slots() as usize))
    }
}
