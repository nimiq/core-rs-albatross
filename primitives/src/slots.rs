extern crate itertools;
///! # Slot allocation primitives
///!
///! This module contains data structures describing the allocation of slots.
///!
///! The graphic below shows how slots relate to validators. In the example we have two
///! validators that produce blocks distributed over a total of 16 slots. Validator #0
///! produces blocks for slots (0 - A). Validator #2 works similarly.
///!
///! ```plain
///!                      +---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+
///!              Slots   | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | A | B | C | D | E | F |
///!                      +---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+
///!          Validators  |             Validator #0                  |    Validator #1   |
///!                      |             SlotBand                      |    SlotBand       |
///!                      +-------------------------------------------+-------------------+
///! ```
///!
use std::collections::{BTreeMap, HashMap};

use std::slice::Iter;

use beserial::{Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};
use nimiq_bls::lazy::LazyPublicKey;
use nimiq_bls::{CompressedPublicKey, PublicKey};

use crate::account::ValidatorId;
use crate::policy::SLOTS;
use std::cmp::max;

/// A validator that owns some slots.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Validator {
    pub validator_id: ValidatorId,
    pub public_key: LazyPublicKey,
    // The start and end slots for this validator. For example, if the slot range is (10,25) then
    // this validator owns the slots from 10 (inclusive) to 25 (exclusive). So it owns 25-10=15 slots.
    pub slot_range: (u16, u16),
}

impl Validator {
    /// Creates a new Validator.
    pub fn new<PK: Into<LazyPublicKey>>(
        validator_id: ValidatorId,
        public_key: PK,
        slot_range: (u16, u16),
    ) -> Self {
        Self {
            validator_id,
            public_key: public_key.into(),
            slot_range,
        }
    }

    /// Returns the number of slots owned by this validator.
    pub fn num_slots(&self) -> u16 {
        self.slot_range.1 - self.slot_range.0
    }
}

/// Identifies a slashed slot by the slot id.
/// Contains the corresponding validator id for reference.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlashedSlot {
    pub slot: u16,
    pub validator_id: ValidatorId,
    /// The `event_block` identifies the block at which the slashable action occurred.
    pub event_block: u32,
}

impl SlashedSlot {
    pub const SIZE: usize = 2 + ValidatorId::SIZE + 4;
}

/// A collection of Validators. This struct is normally used to hold the validators for a specific
/// epoch.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Validators {
    // A vector of validators ordered by their slots. In this case, the slot band of a validator
    // corresponds to their index in the vector.
    pub validators: Vec<Validator>,
    // A mapping of validator ids to their slot bands.
    pub validator_map: HashMap<ValidatorId, u16>,
}

impl Validators {
    /// Creates a new Validators struct given a vector of Validator.
    pub fn new(validators: Vec<Validator>) -> Self {
        let mut validator_map = HashMap::new();

        for (i, validator) in validators.iter().enumerate() {
            validator_map.insert(validator.validator_id.clone(), i as u16);
        }

        Self {
            validators,
            validator_map,
        }
    }

    /// Returns the number of validators contained inside.
    pub fn num_validators(&self) -> usize {
        self.validators.len()
    }

    /// Calculates the slot band of the validator that owns the given slot.
    pub fn get_band_from_slot(&self, slot: u16) -> u16 {
        assert!(slot < SLOTS);

        let mut pivot = self.num_validators() / 2;
        let mut last_pivot = 0usize;
        loop {
            let pivot_diff = if pivot < last_pivot {
                last_pivot - pivot
            } else {
                pivot - last_pivot
            };
            last_pivot = pivot;
            let (start, end) = self.validators[pivot].slot_range;
            if slot < start {
                pivot -= max(pivot_diff / 2, 1);
            } else if slot >= end {
                pivot += max(pivot_diff / 2, 1);
            } else {
                return pivot as u16;
            }
        }
    }

    /// Returns the validator given the slot number.
    pub fn get_validator(&self, slot: u16) -> &Validator {
        &self.validators[self.get_band_from_slot(slot) as usize]
    }

    /// Returns the validator given its ID, if it exists.
    pub fn get_validator_by_id(&self, id: ValidatorId) -> Option<&Validator> {
        let band = *self.validator_map.get(&id)?;
        Some(&self.validators[band as usize])
    }

    /// Returns the public key associated with each slot, in order.
    pub fn to_pks(&self) -> Vec<PublicKey> {
        let mut pks = vec![];

        for validator in self.iter() {
            let pk = *validator.public_key.uncompress().unwrap();

            pks.append(&mut vec![pk; validator.num_slots() as usize]);
        }

        pks
    }

    /// Iterates over the validators.
    pub fn iter(&self) -> Iter<Validator> {
        self.validators.iter()
    }
}

impl Serialize for Validators {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = 0;

        size += Serialize::serialize(&(self.num_validators() as u16), writer)?;

        for validator in self.iter() {
            size += Serialize::serialize(&validator.validator_id, writer)?;
            size += Serialize::serialize(&validator.public_key, writer)?;
            size += Serialize::serialize(&validator.slot_range.0, writer)?;
            size += Serialize::serialize(&validator.slot_range.1, writer)?;
        }

        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        2 + (ValidatorId::SIZE + CompressedPublicKey::SIZE + 2 * 2) * self.num_validators()
    }
}

impl Deserialize for Validators {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let num_validators: u16 = Deserialize::deserialize(reader)?;

        let mut validators = Vec::with_capacity(num_validators as usize);

        for _ in 0..num_validators {
            let validator_id: ValidatorId = Deserialize::deserialize(reader)?;
            let public_key: CompressedPublicKey = Deserialize::deserialize(reader)?;
            let start: u16 = Deserialize::deserialize(reader)?;
            let end: u16 = Deserialize::deserialize(reader)?;
            validators.push(Validator::new(validator_id, public_key, (start, end)));
        }

        Ok(Self::new(validators))
    }
}

/// Builder for slot collection. You can push individual slots into it and it'll compress them
/// into a Validators struct.
#[derive(Clone, Debug, Default)]
pub struct ValidatorsBuilder {
    /// Maps validator id -> (validator key, number of slots)
    validators: BTreeMap<ValidatorId, (LazyPublicKey, u16)>,
}

impl ValidatorsBuilder {
    /// Create a new empty builder.
    pub fn new() -> ValidatorsBuilder {
        ValidatorsBuilder {
            validators: BTreeMap::new(),
        }
    }

    /// Push a new validator slot. This will add one slot to the validator, if it already exists
    pub fn push<PK: Into<LazyPublicKey>>(&mut self, validator_id: ValidatorId, public_key: PK) {
        let (_, num_slots) = self
            .validators
            .entry(validator_id)
            .or_insert_with(|| (public_key.into(), 0));
        *num_slots += 1;
    }

    /// Builds a Validators struct.
    pub fn build(self) -> Validators {
        let mut validators = Vec::new();

        let mut start = 0;

        for (validator_id, (public_key, num_slots)) in self.validators {
            validators.push(Validator::new(
                validator_id,
                public_key,
                (start, start + num_slots),
            ));

            start += num_slots;
        }

        Validators::new(validators)
    }
}
