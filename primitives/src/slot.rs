///! # Slot allocation primitives
///!
///! This module contains data structures describing the allocation of slots between stakes and
///! validators.
///!
///! The graphic below shows how slots relate to stakes and validators. In the example we have two
///! validators that produce blocks for 5 stakes distributed over a total of 16 slots. Validator #0
///! produces blocks for staker #0 (slots 0 - 2) and staker #1 (slots 3 to 9). Validator #2 works
///! similarly.
///!
///! ```plain
///!                      +---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+
///!              Slots   | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | A | B | C | D | E | F |
///!                      +---+---+-------+---+---+---+---+---+-----------+---+---+-------+
///!         StakeSlots   | Staker #0 |        Staker #1          | 3 |   Staker #4   | 5 |
///!        (addresses)   | SlotBand  |        SlotBand           |   |   SlotBand    |   |
///!                      +-----------+---------------------------+-------------------+---+
///!     ValidatorSlots   |             Validator #0                  |    Validator #1   |
///!          (pubkeys)   |             SlotBand                      |    SlotBand       |
///!                      +-------------------------------------------+-------------------+
///! ```
///!
///! # Notes
///!
///! * We use *Stake* and *Staker* interchangably here, since they're always identified by the
///! * staker address.
///!
///! # ToDo
///!
///! * Rename module to `slots`.
///! * Generic `SlotBand<T> { inner: T, num_slots }` ?
///! * `SlotIndex` that maps a `slot_number` to the `SlotBand`.
///! * Rename structs:
///!   * `Validator` -> `ValidatorSlotBand`
///!   * `Validators` -> `ValidatorSlots`
///!   * `SlotAddress` -> `StakerSlotBand`
///!   * `SlotAddresses` -> `StakerSlots`
///!


extern crate nimiq_bls as bls;
extern crate nimiq_keys as keys;
extern crate nimiq_utils as utils;

extern crate itertools;

use std::slice::Iter;
use std::vec::IntoIter;
use std::iter::FromIterator;
use std::collections::BTreeMap;
use std::fmt;

use bitvec::prelude::{BitVec, Msb0};
use itertools::Itertools;

use beserial::{Deserialize, Serialize, ReadBytesExt, WriteBytesExt, SerializingError, SerializeWithLength, DeserializeWithLength, uvar};
use bls::bls12_381::lazy::LazyPublicKey;
use bls::bls12_381::CompressedPublicKey;
use keys::Address;

use crate::policy::SLOTS;


/// Enum to index a slot.
///
/// You can either address a slot directly by its slot number, or by it's band number. The band
/// number than corresponds to either the nth [StakeBand] or [ValidatorBand], depending on which
/// collection it is applied.
///
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotIndex {
    Slot(u16),
    Band(u16),
}


/// A slot that contains the corresponding validator and stake information.
///
/// # ToDo
///
/// * We could include the slot number in here
///
#[derive(Clone, Debug)]
pub struct Slot {
    pub validator_slot: ValidatorSlotBand,
    pub stake_slot: StakeSlotBand,
}

impl Slot {
    pub fn public_key(&self) -> &LazyPublicKey {
        self.validator_slot.public_key()
    }

    pub fn staker_address(&self) -> &Address {
        self.stake_slot.staker_address()
    }

    pub fn reward_address(&self) -> &Address {
        self.stake_slot.reward_address()
    }
}

/// Complete view of all slots
///
/// # ToDo
///
/// * Can we use references to the [ValidatorSlots] and [StakeSlots]? If so, we can just reference
///   them right from the block. Maybe something like *SuperCow* works here?
///
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Slots {
    pub validator_slots: ValidatorSlots,
    pub stake_slots: StakeSlots,
}

impl Slots {
    pub fn new(validator_slots: ValidatorSlots, stake_slots: StakeSlots) -> Self {
        Slots {
            validator_slots,
            stake_slots,
        }
    }

    pub fn get(&self, idx: SlotIndex) -> Option<Slot> {
        Some(Slot {
            validator_slot: self.validator_slots.get(idx)?.clone(),
            stake_slot: self.stake_slots.get(idx)?.clone(),
        })
    }

    /// Returns stake slots with first slot number and the corresponding public key of the
    /// validator slot.
    pub fn combined(&self) -> Vec<(Slot, u16)> {
        let mut combined = Vec::new();
        let mut validator_slots_iter = self.validator_slots.iter().peekable();
        let mut start_stake_slot_number = 0;
        let mut start_validator_slot_number = 0;

        for stake_slot in self.stake_slots.iter() {
            let validator_slot = *validator_slots_iter.peek()
                .expect("Expected validator slot");

            let slot = Slot {
                validator_slot: validator_slot.clone(),
                stake_slot: stake_slot.clone(),
            };
            combined.push((slot, start_stake_slot_number));

            let end_stake_slot_number = start_stake_slot_number + stake_slot.num_slots();
            let end_validator_slot_number = start_validator_slot_number + validator_slot.num_slots();

            if end_stake_slot_number == end_validator_slot_number {
                validator_slots_iter.next();
                start_validator_slot_number = end_validator_slot_number;
            }
            start_stake_slot_number = end_stake_slot_number;
        }

        combined
    }
}


// Deprecated. Check where we really want this conversion
#[deprecated]
impl From<Slots> for ValidatorSlots {
    fn from(slots: Slots) -> Self {
        slots.validator_slots
    }
}

// Deprecated. Check where we really want this conversion
#[deprecated]
impl From<Slots> for StakeSlots {
    fn from(slots: Slots) -> Self {
        slots.stake_slots
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

    /// The total number of slots. This is contant is should always be [policy::SLOTS]
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


/// Builder for slot collection. You can push individial slots into it and it'll compress them
/// into a [ValidatorSlots] and [StakeSlots] collection.
///
#[derive(Clone, Debug, Default)]
pub struct SlotsBuilder {
    /// Maps validator key -> ((staker address, reward address) -> number of slots))
    validators: BTreeMap<LazyPublicKey, BTreeMap<(Address, Option<Address>), u16>>,
}

impl SlotsBuilder {
    /// Push a new validator slot. This will add one slot to the validator and stake.
    ///
    /// # Arguments
    ///
    /// * `public_key` - Public key of validator
    /// * `staker_address` - Address of staker
    /// * `reward_address` - Address where reward will be sent to
    ///
    pub fn push<PK: Into<LazyPublicKey>>(&mut self, public_key: PK, staker_address: Address, reward_address_opt: Option<Address>) {
        let stake_slots = self.validators.entry(public_key.into())
            .or_default();
        let num_slots = stake_slots.entry((staker_address, reward_address_opt))
            .or_default();
        *num_slots += 1;
    }

    pub fn build(self) -> Slots {
        let mut validator_slots = Vec::new();
        let mut stake_slots = Vec::new();

        for (public_key, stakes) in self.validators {
            let mut validator_num_slots = 0;

            for ((staker_address, reward_address_opt), stake_num_slots) in stakes {
                // add stake slot band
                stake_slots.push(StakeSlotBand::new(staker_address, reward_address_opt, stake_num_slots));

                // this stake and the corresponding slots belong to the validator in the outer loop
                validator_num_slots += stake_num_slots;
            }

            // add validator slots
            validator_slots.push(ValidatorSlotBand::new(public_key, validator_num_slots))
        }

        let validator_slots = ValidatorSlots::new(validator_slots);
        let stake_slots = StakeSlots::new(stake_slots);

        Slots::new(validator_slots, stake_slots)
    }
}


/// A validator that owns some slots
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ValidatorSlotBand {
    public_key: LazyPublicKey,
    num_slots: u16,
}

impl ValidatorSlotBand {
    pub fn new<PK: Into<LazyPublicKey>>(public_key: PK, num_slots: u16) -> Self {
        Self {
            public_key: public_key.into(),
            num_slots
        }
    }

    pub fn public_key(&self) -> &LazyPublicKey {
        &self.public_key
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
        Self {
            bands,
            index,
        }
    }

    pub fn get_public_key(&self, idx: SlotIndex) -> Option<&LazyPublicKey> {
        Some(&self.get(idx)?.public_key())
    }

    pub fn find_idx_and_num_slots_by_public_key(&self, public_key: &CompressedPublicKey) -> Option<(u16, u16)> {
        self.bands.iter()
            .find_position(|validator| validator.public_key.compressed() == public_key)
            .map(|(idx, validator)| (idx as u16, validator.num_slots))
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
        let allocation = SlotAllocation::from_iter(self.iter()
            .map(|item| item.num_slots))?;
        size += Serialize::serialize(&allocation, writer)?;

        // serialize collection
        for validator in self.iter() {
            size += Serialize::serialize(&validator.public_key, writer)?;
        }

        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        CompressedPublicKey::SIZE * self.bands.len() + SlotAllocation::SIZE
    }
}

impl Deserialize for ValidatorSlots {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let allocation: SlotAllocation = Deserialize::deserialize(reader)?;
        let num_validators = allocation.num_items();
        let allocation = allocation.as_vec();

        let mut validators = Vec::with_capacity(num_validators);
        for num_slots in allocation {
            let public_key: CompressedPublicKey = Deserialize::deserialize(reader)?;
            validators.push(ValidatorSlotBand::new(public_key, num_slots));
        }

        // This invariant should always hold if SlotAllocation is implemented correctly
        assert_eq!(validators.len(), num_validators);

        Ok(Self::from_iter(validators))
    }
}


/// Occupation of a certain number of slots by an staker address (and reward address)
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct StakeSlotBand {
    pub staker_address: Address,
    pub reward_address_opt: Option<Address>,
    pub num_slots: u16,
}

impl StakeSlotBand {
    pub fn new(staker_address: Address, mut reward_address_opt: Option<Address>, num_slots: u16) -> Self {
        // Create canonical representation
        if Some(&staker_address) == reward_address_opt.as_ref() {
            reward_address_opt = None;
        }

        Self {
            staker_address,
            reward_address_opt,
            num_slots
        }
    }

    pub fn staker_address(&self) -> &Address {
        &self.staker_address
    }

    pub fn reward_address(&self) -> &Address {
        self.reward_address_opt.as_ref()
            .unwrap_or(&self.staker_address)
    }
}

impl SlotBand for StakeSlotBand {
    fn num_slots(&self) -> u16 {
        self.num_slots
    }
}

#[derive(Clone, Debug, Default)]
pub struct StakeSlots {
    bands: Vec<StakeSlotBand>,
    index: SlotIndexTable,
}

impl StakeSlots {
    pub fn new(slots: Vec<StakeSlotBand>) -> Self {
        let index = SlotIndexTable::new(&slots);
        Self {
            bands: slots,
            index
        }
    }

    pub fn get_staker_address(&self, idx: SlotIndex) -> Option<&Address> {
        Some(&self.get(idx)?.staker_address())
    }

    pub fn get_reward_address(&self, idx: SlotIndex) -> Option<&Address> {
        Some(self.get(idx)?.reward_address())
    }

    pub fn iter(&self) -> Iter<StakeSlotBand> {
        self.bands.iter()
    }
}

impl SlotCollection for StakeSlots {
    type SlotBand = StakeSlotBand;
    const TOTAL_SLOTS: u16 = SLOTS;

    fn get_band_number_by_slot_number(&self, slot_number: u16) -> Option<u16> {
        self.index.get(slot_number)
    }

    fn get_by_band_number(&self, band_number: u16) -> Option<&Self::SlotBand> {
        self.bands.get(band_number as usize)
    }

    fn len(&self) -> usize {
        self.bands.len()
    }
}

impl FromIterator<StakeSlotBand> for StakeSlots {
    fn from_iter<T: IntoIterator<Item = StakeSlotBand>>(iter: T) -> Self {
        Self::new(iter.into_iter().collect())
    }
}

impl IntoIterator for StakeSlots {
    type Item = StakeSlotBand;
    type IntoIter = IntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.bands.into_iter()
    }
}

impl PartialEq for StakeSlots {
    fn eq(&self, other: &Self) -> bool {
        // Equality on only the slot bands, not the index
        self.bands.eq(&other.bands)
    }
}

impl Eq for StakeSlots {}

impl Serialize for StakeSlots {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = 0;

        // Get slot allocation from collection
        let allocation = SlotAllocation::from_iter(self.iter()
            .map(|item| item.num_slots))?;
        size += Serialize::serialize(&allocation, writer)?;

        // BitVec enconding which reward addresses are set
        //
        // This is more efficient than serializing the `Option<Address>` for every stake, since
        // that takes 1 byte to encode the presence or absense of that key. Instead we encode
        // it as as BitVec, so it only uses 1/8th of that.
        //
        // TODO: Replace with fixed-size BitVec
        let reward_addresses: BitVec<Msb0, u8> = BitVec::from_iter(self.iter()
            .map(|item| item.reward_address_opt.is_some()));
        SerializeWithLength::serialize::<uvar, W>(&reward_addresses, writer)?;

        // Serialize collection
        for slot_address in self.iter() {
            size += Serialize::serialize(&slot_address.staker_address, writer)?;
            if let Some(reward_address) = &slot_address.reward_address_opt {
                assert_ne!(reward_address, &slot_address.staker_address);
                size += Serialize::serialize(reward_address, writer)?;
            }
        }

        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        // Size of slot allocation
        let mut size = SlotAllocation::SIZE;

        // Size of BitVec that encodes which reward addresses are custom
        // TODO: Replace with fixed-size BitVec
        let reward_addresses: BitVec<Msb0, u8> = BitVec::from_iter(self.iter()
            .map(|item| item.reward_address_opt.is_some()));
        size += SerializeWithLength::serialized_size::<uvar>(&reward_addresses);

        for slot_address in self.iter() {
            size += Serialize::serialized_size(&slot_address.staker_address);
            slot_address.reward_address_opt.as_ref().map(|reward_address| {
                size += Serialize::serialized_size(&reward_address);
            });
        }

        size
    }
}

impl Deserialize for StakeSlots {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        // Deserialize the slot allocation
        let allocation: SlotAllocation = Deserialize::deserialize(reader)?;
        let num_slot_addresses = allocation.num_items();
        let allocation = allocation.as_vec();

        // Deserialize which reward addresses are set
        let reward_addresses: BitVec<Msb0, u8> = DeserializeWithLength::deserialize::<uvar, R>(reader)?;

        // Deserialize addresses
        let mut slot_addresses = Vec::with_capacity(num_slot_addresses);

        for (i, num_slots) in allocation.into_iter().enumerate() {
            let staker_address: Address = Deserialize::deserialize(reader)?;

            let reward_address: Option<Address> = if reward_addresses.get(i).cloned().unwrap_or_default() {
                let address = Deserialize::deserialize(reader)?;
                if address == staker_address {
                    // This is an invalid encoding, as it needlessly bloats the block chain
                    return Err(SerializingError::InvalidEncoding)
                }
                Some(address)
            }
            else {
                None
            };

            slot_addresses.push(StakeSlotBand::new(staker_address, reward_address, num_slots));
        }

        // This invariant should always hold if SlotAllocation is implemented correctly
        assert_eq!(slot_addresses.len(), num_slot_addresses);

        Ok(Self::from_iter(slot_addresses))
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
    index: Vec<u16>
}

impl SlotIndexTable {
    pub fn new<B: SlotBand>(bands: &Vec<B>) -> Self {
        let mut index = Vec::with_capacity(SLOTS as usize);

        for (i, band) in bands.iter().enumerate() {
            for _ in 0 .. band.num_slots() {
                index.push(i as u16);
            }
        }

        assert_eq!(index.len(), SLOTS as usize);

        Self {
            index,
        }
    }

    pub fn get(&self, slot_number: u16) -> Option<u16> {
        self.index.get(slot_number as usize).copied()
    }
}

impl Default for SlotIndexTable {
    fn default() -> Self {
        Self {
            index: vec![]
        }
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

    pub fn from_iter<I: Iterator<Item=u16>>(iter: I) -> Result<Self, SerializingError> {
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
            }
            else {
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
