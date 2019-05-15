use std::fmt;
use std::io;
use std::convert::TryInto;

use keys::Address;
use beserial::{Deserialize, Serialize};
use bls::bls12_381::{CompressedPublicKey, CompressedSignature};
use bls::bls12_381::lazy::LazyPublicKey;
use hash::{Blake2bHash, Hash, SerializeContent};
use primitives::coin::Coin;
use primitives::validators::{Slot, Slots, Validators};
use collections::bitset::BitSet;

use crate::BlockError;
use crate::pbft::PbftProof;
use crate::signed;

pub enum TryIntoError {
    MissingExtrinsics,
    InvalidLength,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MacroBlock {
    pub header: MacroHeader,
    pub justification: Option<PbftProof>,
    pub extrinsics: Option<MacroExtrinsics>
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MacroHeader {
    pub version: u16,

    pub validators: CompressedPublicKeys,

    pub block_number: u32,
    pub view_number: u32,
    pub parent_macro_hash: Blake2bHash,

    pub seed: CompressedSignature,
    pub parent_hash: Blake2bHash,
    pub state_root: Blake2bHash,
    pub extrinsics_root: Blake2bHash,

    pub timestamp: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MacroExtrinsics {
    pub slot_allocation: CompressedAddresses,
    pub slash_fine: Coin,
}

impl TryInto<Slots> for MacroBlock {
    type Error = TryIntoError;

    fn try_into(self) -> Result<Slots, Self::Error> {
        if self.extrinsics.is_none() {
            return Err(TryIntoError::MissingExtrinsics);
        }

        let mut compressed_public_keys = self.header.validators;
        let extrinsics = self.extrinsics.expect("Just checked it above");
        let mut compressed_addresses = extrinsics.slot_allocation;
        let size = compressed_public_keys.slot_allocation.len();

        // A well-formed block will always have the same length for both BitSets and be greater than 0
        if size == 0 || size != compressed_addresses.slot_allocation.len() { return Err(TryIntoError::InvalidLength) };

        // Create the final vector we will be returning
        let mut slots = Vec::with_capacity(size);

        let mut public_key = compressed_public_keys.public_keys.remove(0);

        let mut addresses = compressed_addresses.addresses.remove(0);
        let mut reward_address_opt = if addresses.reward_address == addresses.staker_address { None } else { Some(addresses.reward_address) };
        let mut staker_address = addresses.staker_address;

        slots.push(Slot {
            public_key: public_key.clone(),
            reward_address_opt: reward_address_opt.clone(),
            staker_address: staker_address.clone(),
        });

        for i in 1..size {
            if !compressed_public_keys.slot_allocation.contains(i) {
                public_key = compressed_public_keys.public_keys.remove(0);
            }

            if !compressed_addresses.slot_allocation.contains(i) {
                addresses = compressed_addresses.addresses.remove(0);
                reward_address_opt = if addresses.reward_address == addresses.staker_address { None } else { Some(addresses.reward_address) };
                staker_address = addresses.staker_address;
            }

            slots.push(Slot {
                public_key: public_key.clone(),
                reward_address_opt: reward_address_opt.clone(),
                staker_address: staker_address.clone(),
            });
        }

        let slash_fine = extrinsics.slash_fine;
        Ok(Slots::new(slots, slash_fine))
    }
}

// CHECKME: Check for performance
impl From<Slots> for MacroExtrinsics {
    fn from(mut slots: Slots) -> Self {
        let size = slots.len();
        let mut addresses = Vec::with_capacity(size);
        let mut slot_allocation = BitSet::with_capacity(size);

        let first_slot = slots.remove(0);

        let mut current_reward_address = first_slot.reward_address().clone();
        let mut current_staker_address = first_slot.staker_address;
        let slash_fine = slots.slash_fine().clone();

        for (i, slot) in slots.into_iter().enumerate() {
            if slot.staker_address == current_staker_address && *slot.reward_address() == current_reward_address {
                slot_allocation.insert(i+1);
            } else {
                current_reward_address = slot.reward_address().clone();
                current_staker_address = slot.staker_address.clone();
                addresses.push(SlotAddresses {
                    reward_address: slot.reward_address().clone(),
                    staker_address: slot.staker_address,
                });
            }
        }

        let slot_allocation = CompressedAddresses { addresses, slot_allocation };

        MacroExtrinsics {
            slot_allocation,
            slash_fine,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CompressedPublicKeys {
    #[beserial(len_type(u16))]
    public_keys: Vec<LazyPublicKey>,
    slot_allocation: BitSet,
}

// CHECKME: Check for performance
impl From<Slots> for CompressedPublicKeys {
    fn from(mut slots: Slots) -> Self {
        let size = slots.len();
        let mut public_keys = Vec::with_capacity(size);
        let mut slot_allocation = BitSet::with_capacity(size);

        let mut current_public_key = slots.remove(0).public_key;

        for (i, slot) in slots.into_iter().enumerate() {
            if slot.public_key == current_public_key {
                slot_allocation.insert(i+1);
            } else {
                current_public_key = slot.public_key.clone();
                public_keys.push(slot.public_key);
            }
        }

        Self { public_keys, slot_allocation }
    }
}

impl From<MacroExtrinsics> for Slots {
    fn from(_: MacroExtrinsics) -> Self {
        unimplemented!()
    }
}

// CHECKME: Check for performance
impl From<Validators> for CompressedPublicKeys {
    fn from(validators: Validators) -> Self {
        let size = validators.len();
        let mut public_keys = Vec::with_capacity(size);
        let mut slot_allocation = BitSet::with_capacity(size);

        let mut i = 1;
        for validator in validators {
            public_keys.push(validator.public_key);

            let mut last_set_bit = i + validator.num_slots - 1;
            for j in i..last_set_bit {
                slot_allocation.insert(j as usize);
                last_set_bit = j;
            }
            i = last_set_bit + 2;
        }

        CompressedPublicKeys {
            public_keys,
            slot_allocation,
        }
    }
}

impl From<CompressedPublicKeys> for Validators {
    fn from(_: CompressedPublicKeys) -> Self {
        unimplemented!()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CompressedAddresses {
    #[beserial(len_type(u16))]
    addresses: Vec<SlotAddresses>,
    slot_allocation: BitSet,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SlotAddresses {
    staker_address: Address,
    reward_address: Address,
}

impl signed::Message for MacroHeader {
    const PREFIX: u8 = signed::PREFIX_PBFT_PROPOSAL;
}

impl MacroBlock {
    pub fn verify(&self) -> Result<(), BlockError> {
        if self.header.block_number >= 1 && self.justification.is_none() {
            return Err(BlockError::NoJustification);
        }
        Ok(())
    }

    pub fn is_finalized(&self) -> bool {
        self.justification.is_some()
    }

    pub fn hash(&self) -> Blake2bHash {
        self.header.hash()
    }
}

impl SerializeContent for MacroHeader {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> { Ok(self.serialize(writer)?) }
}

impl Hash for MacroHeader { }

impl SerializeContent for MacroExtrinsics {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> { Ok(self.serialize(writer)?) }
}

// TODO Do we need merkle here?
impl Hash for MacroExtrinsics { }

impl fmt::Display for MacroBlock {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "[#{}, view {}, type Macro]",
               self.header.block_number,
               self.header.view_number)
    }
}
