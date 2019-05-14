use std::fmt;
use std::io;

use beserial::{Deserialize, Serialize};
use bls::bls12_381::{PublicKey, Signature};
use hash::{Blake2bHash, Hash, SerializeContent};
use primitives::policy::TWO_THIRD_VALIDATORS;

use crate::BlockError;
use crate::pbft::UntrustedPbftProof;
use crate::signed;
use primitives::slot::Slot;
use primitives::coin::Coin;
use crate::view_change::ViewChangeProof;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MacroBlock {
    pub header: MacroHeader,
    pub justification: Option<UntrustedPbftProof>,
    pub extrinsics: Option<MacroExtrinsics>
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MacroHeader {
    pub version: u16,

    #[beserial(len_type(u16))]
    pub validators: Vec<ValidatorSlot>,

    pub block_number: u32,
    pub view_number: u32,
    pub parent_macro_hash: Blake2bHash,

    pub seed: Signature,
    pub parent_hash: Blake2bHash,
    pub state_root: Blake2bHash,
    pub extrinsics_root: Blake2bHash,

    pub timestamp: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MacroExtrinsics {
    #[beserial(len_type(u16))]
    pub slot_allocation: Vec<Slot>,
    pub slashing_amount: Coin
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ValidatorSlot {
    pub public_key: PublicKey,
    pub slots: u16
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
