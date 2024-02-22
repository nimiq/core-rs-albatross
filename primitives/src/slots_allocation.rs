//! # Slot allocation primitives
//!
//! This module contains data structures describing the allocation of slots.
//!
//! The graphic below shows how slots relate to validators. In the example we have two
//! validators that produce blocks distributed over a total of 16 slots. Validator #0
//! produces blocks for slots (0 - A). Validator #2 works similarly.
//!
//! ```plain
//!                      +---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+
//!              Slots   | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | A | B | C | D | E | F |
//!                      +---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+
//!          Validators  |             Validator #0                  |    Validator #1   |
//!                      |             SlotBand                      |    SlotBand       |
//!                      +-------------------------------------------+-------------------+
//! ```
use std::{cmp::max, collections::BTreeMap, ops::Range, slice::Iter};

use ark_ec::CurveGroup;
use ark_serialize::CanonicalSerialize;
use nimiq_bls::{lazy::LazyPublicKey as LazyBlsPublicKey, G2Projective, PublicKey as BlsPublicKey};
use nimiq_hash::{Hash, HashOutput};
use nimiq_keys::{Address, Ed25519PublicKey as SchnorrPublicKey};

use crate::{merkle_tree::merkle_tree_construct, policy::Policy};

/// This is the depth of the PKTree circuit.
pub const PK_TREE_DEPTH: usize = 5;
/// This is the number of leaves in the PKTree circuit.
pub const PK_TREE_BREADTH: usize = 2_usize.pow(PK_TREE_DEPTH as u32);

/// A slot that is assigned to a validator.
pub struct Slot {
    /// The number identifying this slot.
    pub number: u16,
    /// The slot band this slot is part of.
    pub band: u16,
    /// The validator owning this slot.
    pub validator: Validator,
}

/// A validator that owns some slots.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde-derive", derive(serde::Serialize, serde::Deserialize))]
pub struct Validator {
    pub address: Address,
    pub voting_key: LazyBlsPublicKey,
    pub signing_key: SchnorrPublicKey,
    // The slot range for this validator. For example, if the slot range is (10,25) then
    // this validator owns the slots from 10 (inclusive) to 25 (exclusive). So it owns 25-10=15 slots.
    pub slots: Range<u16>,
}

impl Validator {
    /// Creates a new Validator.
    pub fn new<TBlsKey: Into<LazyBlsPublicKey>, TSchnorrKey: Into<SchnorrPublicKey>>(
        address: Address,
        voting_key: TBlsKey,
        signing_key: TSchnorrKey,
        slots: Range<u16>,
    ) -> Self {
        Self {
            address,
            voting_key: voting_key.into(),
            signing_key: signing_key.into(),
            slots,
        }
    }

    /// Returns the number of slots owned by this validator.
    pub fn num_slots(&self) -> u16 {
        self.slots.len() as u16
    }
}

/// Identifies a penalized slot by the slot id.
/// Contains the corresponding validator id for reference.
#[cfg_attr(feature = "serde-derive", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PenalizedSlot {
    pub slot: u16,
    pub validator_address: Address,
    /// The `offense_event_block` identifies the block at which the penalizable action occurred.
    pub offense_event_block: u32,
}

/// Identifies a jail slot by the validator's address.
#[cfg_attr(feature = "serde-derive", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JailedValidator {
    /// All slots to be jailed.
    pub slots: Range<u16>,
    pub validator_address: Address,
    /// The `offense_event_block` identifies the block at which the malicious behaviour occurred.
    pub offense_event_block: u32,
}

/// A collection of Validators. This struct is normally used to hold the validators for a specific
/// epoch.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Validators {
    // A vector of validators ordered by their slots. In this case, the slot band of a validator
    // corresponds to their index in the vector.
    pub validators: Vec<Validator>,
    // A mapping of validator ids to their slot bands.
    pub validator_map: BTreeMap<Address, u16>,
}

impl Validators {
    /// Creates a new Validators struct given a vector of Validator.
    pub fn new(validators: Vec<Validator>) -> Self {
        let mut validator_map = BTreeMap::new();

        for (i, validator) in validators.iter().enumerate() {
            validator_map.insert(validator.address.clone(), i as u16);
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
        assert!(slot < Policy::SLOTS);

        let mut pivot = self.num_validators() / 2;
        let mut last_pivot = 0usize;
        loop {
            let pivot_diff = if pivot < last_pivot {
                last_pivot - pivot
            } else {
                pivot - last_pivot
            };
            last_pivot = pivot;
            let slots = self.validators[pivot].slots.clone();
            if slot < slots.start {
                pivot -= max(pivot_diff / 2, 1);
            } else if slot >= slots.end {
                pivot += max(pivot_diff / 2, 1);
            } else {
                return pivot as u16;
            }
        }
    }

    /// Returns the validator given the slot number.
    pub fn get_validator_by_slot_number(&self, slot_number: u16) -> &Validator {
        &self.validators[self.get_band_from_slot(slot_number) as usize]
    }

    /// Returns the validator given the slot band.
    pub fn get_validator_by_slot_band(&self, slot_band: u16) -> &Validator {
        &self.validators[slot_band as usize]
    }

    /// Returns the validator given its address, if it exists.
    pub fn get_validator_by_address(&self, address: &Address) -> Option<&Validator> {
        let band = *self.validator_map.get(address)?;
        Some(&self.validators[band as usize])
    }

    /// Returns the slot band of a validator given its address, if it exists.
    pub fn get_slot_band_by_address(&self, address: &Address) -> Option<u16> {
        self.validator_map.get(address).cloned()
    }

    /// Returns the G2 projective associated with each slot, in order.
    pub fn voting_keys_g2(&self) -> Vec<G2Projective> {
        self.voting_keys().iter().map(|pk| pk.public_key).collect()
    }

    /// Returns the voting key associated with each slot, in order.
    pub fn voting_keys(&self) -> Vec<BlsPublicKey> {
        let mut pks = vec![];

        for validator in self.iter() {
            let pk = *validator.voting_key.uncompress().unwrap();
            pks.append(&mut vec![pk; validator.num_slots() as usize]);
        }

        pks
    }

    /// Iterates over the validators.
    pub fn iter(&self) -> Iter<Validator> {
        self.validators.iter()
    }
}

impl Hash for Validators {
    /// This function is meant to calculate the public key tree "off-circuit". Generating the public key
    /// tree with this function guarantees that it is compatible with the ZK circuit.
    // These should be removed once pk_tree_root does something different than returning default
    fn hash<H: HashOutput>(&self) -> H {
        let public_keys = self.voting_keys_g2();

        // Checking that the number of public keys is equal to the number of validator slots.
        assert_eq!(public_keys.len(), Policy::SLOTS as usize);

        // Checking that the number of public keys is a multiple of the number of leaves.
        assert_eq!(public_keys.len() % PK_TREE_BREADTH, 0);

        // Serialize the public keys into bits.
        #[cfg(not(feature = "parallel"))]
        let iter = public_keys.iter();
        #[cfg(feature = "parallel")]
        let iter = self.public_keys.par_iter();
        let bytes: Vec<u8> = iter
            .flat_map(|pk| {
                let mut buffer = [0u8; 285];
                CanonicalSerialize::serialize_compressed(&pk.into_affine(), &mut &mut buffer[..])
                    .unwrap();
                buffer.to_vec()
            })
            .collect();

        // Chunk the bits into the number of leaves.
        let mut inputs = Vec::new();

        for i in 0..PK_TREE_BREADTH {
            inputs.push(
                bytes[i * bytes.len() / PK_TREE_BREADTH..(i + 1) * bytes.len() / PK_TREE_BREADTH]
                    .to_vec(),
            );
        }

        // Calculate the merkle tree root.
        merkle_tree_construct(inputs)
    }
}

/// Builder for slot collection. You can push individual slots into it and it'll compress them
/// into a Validators struct.
#[derive(Clone, Debug, Default)]
pub struct ValidatorsBuilder {
    /// Maps validator address -> (voting key, signing key, number of slots)
    validators: BTreeMap<Address, (LazyBlsPublicKey, SchnorrPublicKey, u16)>,
}

impl ValidatorsBuilder {
    /// Create a new empty builder.
    pub fn new() -> ValidatorsBuilder {
        ValidatorsBuilder {
            validators: BTreeMap::new(),
        }
    }

    /// Push a new validator slot. This will add one slot to the validator, if it already exists
    pub fn push<TBlsKey: Into<LazyBlsPublicKey>, TSchnorrKey: Into<SchnorrPublicKey>>(
        &mut self,
        validator_address: Address,
        voting_key: TBlsKey,
        signing_key: TSchnorrKey,
    ) {
        let (_, _, num_slots) = self
            .validators
            .entry(validator_address)
            .or_insert_with(|| (voting_key.into(), signing_key.into(), 0));
        *num_slots += 1;
    }

    /// Builds a Validators struct.
    pub fn build(self) -> Validators {
        let mut validators = Vec::new();
        let mut start_slot = 0;

        for (validator_address, (voting_key, signing_key, num_slots)) in self.validators {
            validators.push(Validator::new(
                validator_address,
                voting_key,
                signing_key,
                start_slot..(start_slot + num_slots),
            ));
            start_slot += num_slots;
        }

        Validators::new(validators)
    }
}

#[cfg(feature = "serde-derive")]
mod serde_derive {
    use serde::{
        de::{Deserialize, Deserializer},
        ser::{Serialize, Serializer},
    };

    use super::{Validator, Validators};

    impl Serialize for Validators {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            Serialize::serialize(&self.validators, serializer)
        }
    }

    impl<'de> Deserialize<'de> for Validators {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            let validators: Vec<Validator> = Deserialize::deserialize(deserializer)?;
            Ok(Self::new(validators))
        }
    }
}
