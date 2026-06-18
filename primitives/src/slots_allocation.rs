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
use std::{cmp::Ordering, collections::BTreeMap, fmt, ops::Range, slice::Iter};

use nimiq_bls::{G2Projective, LazyPublicKey as LazyBlsPublicKey, PublicKey as BlsPublicKey};
use nimiq_hash::{Hash, HashOutput};
use nimiq_keys::{Address, Ed25519PublicKey as SchnorrPublicKey};
#[cfg(feature = "parallel")]
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};

use crate::{merkle_tree::merkle_tree_construct, policy::Policy};

/// Error returned when a validator has an invalid BLS voting key that cannot be decompressed.
#[derive(Clone, Debug)]
pub struct InvalidValidatorsError;

impl fmt::Display for InvalidValidatorsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Invalid BLS voting key for validator")
    }
}

impl std::error::Error for InvalidValidatorsError {}

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
#[cfg_attr(
    feature = "serde-derive",
    derive(
        nimiq_serde::Deserialize,
        nimiq_serde::Serialize,
        nimiq_serde::SerializedMaxSize,
    )
)]
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
    ///
    /// ## Panic
    /// This function requires the slot to be within bounds. If it is not this function will panic.
    pub fn get_band_from_slot(&self, slot: u16) -> u16 {
        assert!(slot < Policy::SLOTS);

        #[allow(clippy::ok_expect)]
        self.validators
            .binary_search_by(|validator| {
                if validator.slots.end <= slot {
                    Ordering::Less
                } else if validator.slots.start > slot {
                    Ordering::Greater
                } else {
                    Ordering::Equal
                }
            })
            .ok()
            .expect("validator slot band not found")
            .try_into()
            .unwrap()
    }

    /// Returns the validator given the slot number.
    ///
    /// ## Panic
    /// This function requires the slot_number to be within bounds. If it is not this function will panic.
    pub fn get_validator_by_slot_number(&self, slot_number: u16) -> &Validator {
        &self.validators[self.get_band_from_slot(slot_number) as usize]
    }

    /// Returns the validator given the slot band, if it exists.
    pub fn get_validator_by_slot_band(&self, slot_band: u16) -> Option<&Validator> {
        self.validators.get(slot_band as usize)
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
    pub fn voting_keys_g2(&self) -> Result<Vec<G2Projective>, InvalidValidatorsError> {
        Ok(self.voting_keys()?.iter().map(|pk| pk.public_key).collect())
    }

    /// Returns the voting key associated with each slot, in order.
    pub fn voting_keys(&self) -> Result<Vec<BlsPublicKey>, InvalidValidatorsError> {
        let mut pks = vec![];

        for validator in self.iter() {
            let pk = *validator
                .voting_key
                .uncompress()
                .ok_or(InvalidValidatorsError)?;
            pks.append(&mut vec![pk; validator.num_slots() as usize]);
        }

        Ok(pks)
    }

    /// Checks that all validator voting keys can be decompressed and that the
    /// validator set structure is valid (correct number of slots, compatible with ZK circuit).
    pub fn validate_keys(&self) -> Result<(), InvalidValidatorsError> {
        let mut total_slots: u16 = 0;

        // Check that all keys can be decompressed and count total slots in one pass
        for validator in self.iter() {
            validator
                .voting_key
                .uncompress()
                .ok_or(InvalidValidatorsError)?;
            // Use `checked_add` so a crafted validator set cannot silently wrap the
            // running total around `u16` (overflow checks are disabled in the release
            // profile). Any overflow implies far more than `Policy::SLOTS` slots, so the
            // set is invalid; rejecting here prevents a wrapped total from slipping
            // through and later mismatching the `usize` total computed in
            // `Validators::hash()`
            total_slots = total_slots
                .checked_add(validator.num_slots())
                .ok_or(InvalidValidatorsError)?;
        }

        // Check structural invariants that would cause panics in Hash implementation

        // Must have exactly Policy::SLOTS total slots
        if total_slots != Policy::SLOTS {
            return Err(InvalidValidatorsError);
        }

        // Must be a multiple of PK_TREE_BREADTH for ZK circuit compatibility
        if !(total_slots as usize).is_multiple_of(PK_TREE_BREADTH) {
            return Err(InvalidValidatorsError);
        }

        Ok(())
    }

    /// Iterates over the validators.
    pub fn iter(&self) -> Iter<'_, Validator> {
        self.validators.iter()
    }
}

impl Hash for Validators {
    /// This function is meant to calculate the public key tree "off-circuit". Generating the public key
    /// tree with this function guarantees that it is compatible with the ZK circuit.
    fn hash<H: HashOutput>(&self) -> H {
        // Use the raw compressed key bytes directly instead of decompressing to
        // G2Projective and re-serializing. For valid keys, the canonical compressed
        // serialization is a fixed-point of the uncompress -> serialize_compressed
        // round-trip, so this produces identical output. For invalid keys, this
        // avoids the panic that would occur during decompression.
        #[cfg(not(feature = "parallel"))]
        let iter = self.validators.iter();
        #[cfg(feature = "parallel")]
        let iter = self.validators.par_iter();
        let bytes: Vec<u8> = iter
            .flat_map(|validator| {
                let key_bytes = validator.voting_key.compressed().as_ref();
                key_bytes.repeat(validator.num_slots() as usize)
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
    use nimiq_serde::SerializedMaxSize;
    use serde::{
        de::{Deserialize, Deserializer},
        ser::{Serialize, Serializer},
    };

    use super::{Validator, Validators};
    use crate::policy::Policy;

    impl SerializedMaxSize for Validators {
        const MAX_SIZE: usize =
            nimiq_serde::seq_max_size(Validator::MAX_SIZE, Policy::SLOTS as usize);
    }

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
            use serde::de::Error as _;

            let validators: Vec<Validator> = Deserialize::deserialize(deserializer)?;

            // Reject oversized sets at the untrusted boundary: a tiny `slots:
            // Range<u16>` can declare huge slot counts that `Validators::hash()`
            // would materialize into GBs. A real set sums to exactly `Policy::SLOTS`.
            // `checked_add` guards the `u16` total (release builds skip overflow checks).
            let mut total_slots: u16 = 0;
            for validator in &validators {
                total_slots = total_slots
                    .checked_add(validator.num_slots())
                    .ok_or_else(|| D::Error::custom("validator slot total overflows u16"))?;
            }
            if total_slots > Policy::SLOTS {
                return Err(D::Error::custom(
                    "validator slot total exceeds Policy::SLOTS",
                ));
            }

            Ok(Self::new(validators))
        }
    }
}
