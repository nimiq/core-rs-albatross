use std::fmt;
use std::io;

use keys::Address;
use beserial::{Deserialize, Serialize};
use bls::bls12_381::{CompressedPublicKey, CompressedSignature};
use bls::bls12_381::lazy::LazyPublicKey;
use hash::{Blake2bHash, Hash, SerializeContent};
use primitives::coin::Coin;
use primitives::validators::{Slots, Validators};
use collections::bitset::BitSet;

use crate::BlockError;
use crate::pbft::PbftProof;
use crate::signed;

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
