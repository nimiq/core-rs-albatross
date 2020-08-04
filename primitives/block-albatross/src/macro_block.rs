use std::convert::TryInto;
use std::fmt;
use std::io;

use failure::Fail;

use beserial::{Deserialize, Serialize};
use collections::bitset::BitSet;
use hash::{Blake2bHash, Hash, SerializeContent};
use primitives::slot::{Slots, ValidatorSlots};
use vrf::VrfSeed;

use crate::pbft::PbftProof;
use crate::signed;
use crate::BlockError;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MacroBlock {
    pub header: MacroHeader,
    pub justification: Option<PbftProof>,
    pub extrinsics: Option<MacroExtrinsics>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MacroHeader {
    pub version: u16,

    /// Slots with validator information, i.e. their public key & reward address.
    pub validators: Option<ValidatorSlots>,

    pub block_number: u32,
    pub view_number: u32,
    pub parent_macro_hash: Blake2bHash,
    pub parent_election_hash: Blake2bHash,

    pub seed: VrfSeed,
    pub parent_hash: Blake2bHash,
    pub state_root: Blake2bHash,
    pub extrinsics_root: Blake2bHash,

    /// A merkle root over all transactions from the previous epoch.
    pub transactions_root: Blake2bHash,

    pub timestamp: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MacroExtrinsics {
    /// The final list of slashes from the previous epoch.
    pub slashed_set: BitSet,
    /// the slashed set of the current epoch.
    /// None for election blocks as the slashed set of the current epoch is always an empty BitSet for those
    pub current_slashed_set: Option<BitSet>,
    #[beserial(len_type(u8))]
    pub extra_data: Vec<u8>,
}

impl MacroBlock {
    pub fn verify(&self) -> Result<(), BlockError> {
        if self.header.block_number >= 1 && self.justification.is_none() {
            return Err(BlockError::NoJustification);
        }
        Ok(())
    }

    pub fn hash(&self) -> Blake2bHash {
        self.header.hash()
    }

    /// Returns wether or not this macro block header contains an election
    pub fn is_election_block(&self) -> bool {
        self.header.validators.is_some()
    }
}

// CHECKME: Check for performance
impl MacroExtrinsics {
    pub fn from_slashed_set(slashed_set: BitSet, current_slashed_set: Option<BitSet>) -> Self {
        MacroExtrinsics {
            slashed_set,
            current_slashed_set,
            extra_data: vec![],
        }
    }
}

impl signed::Message for MacroHeader {
    const PREFIX: u8 = signed::PREFIX_PBFT_PROPOSAL;
}

impl SerializeContent for MacroHeader {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> {
        Ok(self.serialize(writer)?)
    }
}

#[allow(clippy::derive_hash_xor_eq)] // TODO: Shouldn't be necessary
impl Hash for MacroHeader {}

impl SerializeContent for MacroExtrinsics {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> {
        Ok(self.serialize(writer)?)
    }
}

// TODO Do we need merkle here?
#[allow(clippy::derive_hash_xor_eq)] // TODO: Shouldn't be necessary
impl Hash for MacroExtrinsics {}

impl fmt::Display for MacroBlock {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(
            f,
            "[#{}, view {}, type Macro]",
            self.header.block_number, self.header.view_number
        )
    }
}

#[derive(Clone, Debug, Fail)]
pub enum IntoSlotsError {
    #[fail(display = "Extrinsics missing in macro block")]
    MissingExtrinsics,
    #[fail(display = "Not an election macro block")]
    NoElection,
}

impl TryInto<Slots> for MacroBlock {
    type Error = IntoSlotsError;

    fn try_into(self) -> Result<Slots, Self::Error> {
        if let Some(validator_slots) = self.header.validators {
            Ok(Slots::new(validator_slots))
        } else {
            Err(IntoSlotsError::NoElection)
        }
    }
}
