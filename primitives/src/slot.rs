extern crate itertools;
///! # Slot allocation primitives
///!
///! This module contains data structures describing the allocation of slots.
///!
///! The graphic below shows how slots relate to validators. In the example we have two
///! validators that produce blocks distributed over a total of 16 slots. Validator #0
///! produces blocks for slots 0 - A). Validator #2 works similarly.
///!
///! ```plain
///!                      +---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+
///!              Slots   | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | A | B | C | D | E | F |
///!                      +---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+
///!     ValidatorSlots   |             Validator #0                  |    Validator #1   |
///!          (pubkeys)   |             SlotBand                      |    SlotBand       |
///!                      +-------------------------------------------+-------------------+
///! ```
///!
///! # Notes
///!
///! * We use *Stake* and *Staker* interchangeably here, since they're always identified by the
///! * staker address.
///!
extern crate nimiq_bls as bls;
extern crate nimiq_keys as keys;
extern crate nimiq_utils as utils;

use std::collections::BTreeMap;
use std::fmt;
use std::iter::FromIterator;
use std::slice::Iter;
use std::vec::IntoIter;

use bitvec::order::Msb0;
use bitvec::prelude::BitVec;
use itertools::Itertools;

use beserial::{Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};
use bls::lazy::LazyPublicKey;
use bls::CompressedPublicKey;
use keys::Address;

use crate::account::ValidatorId;
use crate::policy::SLOTS;

/// Enum to index a slot.
///
/// You can either address a slot directly by its slot number, or by it's band number. The band
/// number than corresponds to the nth [ValidatorBand].
///
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotIndex {
    Slot(u16),
    Band(u16),
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

/// A slot that contains the corresponding validator information.
///
/// # ToDo
///
/// * We could include the slot number in here
///
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Slot {
    pub validator_slot: ValidatorSlotBand,
}

impl Slot {
    pub fn validator_id(&self) -> &ValidatorId {
        self.validator_slot.validator_id()
    }

    pub fn public_key(&self) -> &LazyPublicKey {
        self.validator_slot.public_key()
    }

    pub fn reward_address(&self) -> &Address {
        self.validator_slot.reward_address()
    }
}

/// Complete view of all slots
///
/// # ToDo
///
/// * Can we use references to the [ValidatorSlots]? If so, we can just reference
///   them right from the block. Maybe something like *SuperCow* works here?
///
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Slots {
    pub validator_slots: ValidatorSlots,
}

impl Slots {
    pub fn new(validator_slots: ValidatorSlots) -> Self {
        Slots { validator_slots }
    }

    pub fn get(&self, idx: SlotIndex) -> Option<Slot> {
        Some(Slot {
            validator_slot: self.validator_slots.get(idx)?.clone(),
        })
    }

    /// Returns validator slots with first slot number and the corresponding public key of the
    /// validator slot.
    pub fn combined(&self) -> Vec<(Slot, u16)> {
        let mut combined = Vec::new();
        let mut start_validator_slot_number = 0;

        for validator_slot in self.validator_slots.iter() {
            let slot = Slot {
                validator_slot: validator_slot.clone(),
            };
            combined.push((slot, start_validator_slot_number));

            start_validator_slot_number += validator_slot.num_slots();
        }

        combined
    }
}

// Deprecated. Check where we really want this conversion
impl From<Slots> for ValidatorSlots {
    fn from(slots: Slots) -> Self {
        slots.validator_slots
    }
}

/// Trait implemented by something that can occupy a band of slots.
pub trait SlotBand {
    /// Returns the number of slots this band occupies.
    fn num_slots(&self) -> u16;
}

/// Trait for collections of slots.
pub trait SlotCollection {
    /// The kind of [SlotBand] that is stored in this collection.
    type SlotBand: SlotBand;

    /// The total number of slots. This is constant is should always be [policy::SLOTS]
    const TOTAL_SLOTS: u16;

    fn get_band_number_by_slot_number(&self, slot_number: u16) -> Option<u16>;

    /// Returns the [SlotBand] given its band number. So this is either the stake index
    /// or validator index (a.k.a `pk_idx`, `validator_id`, `signer_idx`).
    fn get_by_band_number(&self, band_number: u16) -> Option<&Self::SlotBand>;

    /// Returns the [SlotBand] given the slot number.
    fn get_by_slot_number(&self, slot_number: u16) -> Option<&Self::SlotBand> {
        let band_number = self.get_band_number_by_slot_number(slot_number)?;
        self.get_by_band_number(band_number)
    }

    /// Returns the [SlotBand] either by slot number or band number.
    fn get(&self, idx: SlotIndex) -> Option<&Self::SlotBand> {
        match idx {
            SlotIndex::Slot(slot_number) => self.get_by_slot_number(slot_number),
            SlotIndex::Band(band_number) => self.get_by_band_number(band_number),
        }
    }

    /// Returns the number of slots in a slot band, given the slot index.
    fn get_num_slots(&self, idx: SlotIndex) -> Option<u16> {
        Some(self.get(idx)?.num_slots())
    }

    /// Returns the number of slot bands in this collection
    fn len(&self) -> usize;
}

/// Builder for slot collection. You can push individual slots into it and it'll compress them
/// into a [ValidatorSlots] collection.
#[derive(Clone, Debug, Default)]
pub struct SlotsBuilder {
    /// Maps validator id -> (validator key, reward address, number of slots)
    validators: BTreeMap<ValidatorId, (LazyPublicKey, Address, u16)>,
}

impl SlotsBuilder {
    /// Push a new validator slot. This will add one slot to the validator.
    ///
    /// # Arguments
    ///
    /// * `public_key` - Public key of validator
    /// * `reward_address` - Address where reward will be sent to
    ///
    pub fn push<PK: Into<LazyPublicKey>>(
        &mut self,
        validator_id: ValidatorId,
        public_key: PK,
        reward_address: &Address,
    ) {
        let (_, _, num_slots) = self
            .validators
            .entry(validator_id)
            .or_insert_with(|| (public_key.into(), reward_address.clone(), 0));
        *num_slots += 1;
    }

    pub fn build(self) -> Slots {
        let mut validator_slots = Vec::new();

        for (validator_id, (public_key, reward_address, num_slots)) in self.validators {
            // Add validator slots.
            validator_slots.push(ValidatorSlotBand::new(
                validator_id,
                public_key,
                reward_address,
                num_slots,
            ));
        }

        let validator_slots = ValidatorSlots::new(validator_slots);

        Slots::new(validator_slots)
    }
}

/// A validator that owns some slots
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ValidatorSlotBand {
    validator_id: ValidatorId,
    public_key: LazyPublicKey,
    reward_address: Address,
    num_slots: u16,
}

impl ValidatorSlotBand {
    pub fn new<PK: Into<LazyPublicKey>>(
        validator_id: ValidatorId,
        public_key: PK,
        reward_address: Address,
        num_slots: u16,
    ) -> Self {
        Self {
            validator_id,
            public_key: public_key.into(),
            reward_address,
            num_slots,
        }
    }

    pub fn validator_id(&self) -> &ValidatorId {
        &self.validator_id
    }

    pub fn public_key(&self) -> &LazyPublicKey {
        &self.public_key
    }

    pub fn reward_address(&self) -> &Address {
        &self.reward_address
    }
}

impl SlotBand for ValidatorSlotBand {
    fn num_slots(&self) -> u16 {
        self.num_slots
    }
}

#[derive(Clone, Debug, Default)]
pub struct ValidatorSlots {
    bands: Vec<ValidatorSlotBand>,
    index: SlotIndexTable,
}

impl ValidatorSlots {
    pub fn new(bands: Vec<ValidatorSlotBand>) -> Self {
        let index = SlotIndexTable::new(&bands);
        Self { bands, index }
    }

    pub fn get_public_key(&self, idx: SlotIndex) -> Option<&LazyPublicKey> {
        Some(&self.get(idx)?.public_key())
    }

    pub fn find_idx_and_num_slots_by_public_key(
        &self,
        public_key: &CompressedPublicKey,
    ) -> Option<(u16, u16)> {
        log::info!("public_key = {:?}", public_key);

        self.bands
            .iter()
            .find_position(|validator| validator.public_key.compressed() == public_key)
            .map(|(idx, validator)| (idx as u16, validator.num_slots))
    }

    /// for a given band `idx` returns the curresponding slots or empty vector if there are none.
    pub fn get_slots(&self, idx: u16) -> Vec<u16> {
        if idx as usize >= self.bands.len() {
            return vec![];
        } else {
            let mut slot_index = 0;
            for i in 0..=idx {
                if i != idx {
                    slot_index += self.bands.get(i as usize).unwrap().num_slots();
                } else {
                    return (slot_index
                        ..(slot_index + self.bands.get(i as usize).unwrap().num_slots()))
                        .collect();
                }
            }
        }
        vec![]
    }

    pub fn iter(&self) -> Iter<ValidatorSlotBand> {
        self.bands.iter()
    }
}

impl SlotCollection for ValidatorSlots {
    type SlotBand = ValidatorSlotBand;
    const TOTAL_SLOTS: u16 = SLOTS;

    fn get_band_number_by_slot_number(&self, slot_number: u16) -> Option<u16> {
        self.index.get(slot_number)
    }

    fn get_by_band_number(&self, i: u16) -> Option<&Self::SlotBand> {
        self.bands.get(i as usize)
    }

    fn len(&self) -> usize {
        self.bands.len()
    }
}

impl FromIterator<ValidatorSlotBand> for ValidatorSlots {
    fn from_iter<T: IntoIterator<Item = ValidatorSlotBand>>(iter: T) -> Self {
        Self::new(iter.into_iter().collect())
    }
}

impl IntoIterator for ValidatorSlots {
    type Item = ValidatorSlotBand;
    type IntoIter = IntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.bands.into_iter()
    }
}

impl PartialEq for ValidatorSlots {
    fn eq(&self, other: &Self) -> bool {
        // Equality on only the slot bands, not the index
        self.bands.eq(&other.bands)
    }
}

impl Eq for ValidatorSlots {}

impl Serialize for ValidatorSlots {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = 0;

        // get slot allocation from collection
        let allocation = SlotAllocation::from_iter(self.iter().map(|item| item.num_slots))?;
        size += Serialize::serialize(&allocation, writer)?;

        // serialize collection
        for validator in self.iter() {
            size += Serialize::serialize(&validator.validator_id, writer)?;
            size += Serialize::serialize(&validator.public_key, writer)?;
            size += Serialize::serialize(&validator.reward_address, writer)?;
        }

        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        (ValidatorId::SIZE + CompressedPublicKey::SIZE + Address::SIZE) * self.bands.len()
            + SlotAllocation::SIZE
    }
}

impl Deserialize for ValidatorSlots {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let allocation: SlotAllocation = Deserialize::deserialize(reader)?;
        let num_validators = allocation.num_items();
        let allocation = allocation.as_vec();

        let mut validators = Vec::with_capacity(num_validators);
        for num_slots in allocation {
            let validator_id: ValidatorId = Deserialize::deserialize(reader)?;
            let public_key: CompressedPublicKey = Deserialize::deserialize(reader)?;
            let reward_address: Address = Deserialize::deserialize(reader)?;
            validators.push(ValidatorSlotBand::new(
                validator_id,
                public_key,
                reward_address,
                num_slots,
            ));
        }

        // This invariant should always hold if SlotAllocation is implemented correctly
        assert_eq!(validators.len(), num_validators);

        Ok(Self::from_iter(validators))
    }
}

/// Mapping from slot number to [SlotBand].
///
/// Internally this is just a [std::vec::Vec] with the indices into a [std::vec::Vec] with
/// [SlotBand]s.
///
/// # ToDo
///
/// * Use a [nimiq_collections::segment_tree::SegmentTree]
///
#[derive(Clone)]
struct SlotIndexTable {
    index: Vec<u16>,
}

impl SlotIndexTable {
    pub fn new<B: SlotBand>(bands: &Vec<B>) -> Self {
        let mut index = Vec::with_capacity(SLOTS as usize);

        for (i, band) in bands.iter().enumerate() {
            for _ in 0..band.num_slots() {
                index.push(i as u16);
            }
        }

        assert_eq!(index.len(), SLOTS as usize);

        Self { index }
    }

    pub fn get(&self, slot_number: u16) -> Option<u16> {
        self.index.get(slot_number as usize).copied()
    }
}

impl Default for SlotIndexTable {
    fn default() -> Self {
        Self { index: vec![] }
    }
}

impl fmt::Debug for SlotIndexTable {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "SlotIndexTable {{ ... }}")
    }
}

/// A compressed representation of repeated objects. This is used to encode the slot allocation
/// of something that is a `NumSlotsCollection`
struct SlotAllocation {
    bits: BitVec<Msb0, u8>,
}

impl SlotAllocation {
    /// Size in bytes - number of slots divided by 8 and rounded up.
    const SIZE: usize = ((SLOTS as usize) + 8 - 1) / 8;

    pub fn from_iter<I: Iterator<Item = u16>>(iter: I) -> Result<Self, SerializingError> {
        let mut bits = BitVec::with_capacity(SLOTS as usize);
        bits.resize(SLOTS as usize, false);

        let mut i = 0;

        for num_slots in iter {
            // This is not allowed
            if num_slots == 0 {
                return Err(SerializingError::InvalidValue);
            }

            // Set bit to encode a new validator and 1 slot
            bits.set(i, true);

            // Skip num_slots bits, thus num_slots - 1 bits are left 0, which is one 0 for each
            // repitition
            i += num_slots as usize;
        }

        assert_eq!(i, SLOTS as usize);

        Ok(Self { bits })
    }

    pub fn as_vec(&self) -> Vec<u16> {
        let mut num_slots = Vec::new();
        let mut count = 0;
        let mut total = 0;

        for b in &self.bits {
            if *b && count > 0 {
                num_slots.push(count);
                total += count;
                count = 1;
            } else {
                count += 1;
            }
        }

        total += count;
        num_slots.push(count);

        // Verify that the cumulative number of slots is `policy::SLOTS`.
        // NOTE: Because of the nature of the encoding this always holds if the implementation of
        //       `SlotAllocation` is correct.
        assert_eq!(total, SLOTS);

        num_slots
    }

    pub fn num_items(&self) -> usize {
        self.bits.count_ones()
    }
}

impl Serialize for SlotAllocation {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let slice = self.bits.as_slice();
        writer.write_all(slice)?;
        Ok(slice.len())
    }

    fn serialized_size(&self) -> usize {
        Self::SIZE
    }
}

impl Deserialize for SlotAllocation {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        // Allocate buffer for BitVec
        let mut buf = Vec::with_capacity(Self::SIZE);
        buf.resize(Self::SIZE, 0_u8);

        // Read and create BitVec
        reader.read_exact(buf.as_mut_slice())?;
        let mut bits = BitVec::from_vec(buf);

        // We can end up with a BitVec that is slightly larger, if the original BitVec's length
        // wasn't a multiple of 8. Check that and resize to exact length
        assert!(bits.len() >= SLOTS as usize);
        bits.resize(SLOTS as usize, false);

        // Parse into slot allocation
        let allocation = Self { bits };

        Ok(allocation)
    }
}
