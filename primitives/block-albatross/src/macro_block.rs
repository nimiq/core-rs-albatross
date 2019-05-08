use std::io;
use std::fmt;

use crate::view_change::ViewChangeProof;
use beserial::{Deserialize, Serialize};
use hash::{Blake2bHash, Hash, SerializeContent};
use bls::bls12_381::{PublicKey, Signature};
use crate::signed;
use crate::pbft::PbftProof;
use primitives::policy::TWO_THIRD_VALIDATORS;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MacroBlock {
    pub header: MacroHeader,
    pub justification: Option<MacroJustification>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MacroHeader {
    pub version: u16,

    // Digest
    #[beserial(len_type(u16))]
    pub validators: Vec<PublicKey>,
    pub block_number: u32,
    pub view_number: u32,
    pub parent_macro_hash: Blake2bHash,

    pub seed: Signature,
    pub parent_hash: Blake2bHash,
    pub state_root: Blake2bHash,

    pub timestamp: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MacroJustification {
    pub pbft_proof: PbftProof,
    pub view_change_proof: Option<ViewChangeProof>,
}

impl signed::Message for MacroHeader {
    const PREFIX: u8 = signed::PREFIX_PBFT_PROPOSAL;
}

impl MacroBlock {
    pub fn verify(&self) -> bool {
        if self.header.block_number >= 1 && self.justification.is_none() {
            return false;
        }

        if let Some(justification) = &self.justification {
            if !justification.verify(self.hash(), TWO_THIRD_VALIDATORS) {
                return false;
            }
        }
        return true;
    }

    pub fn is_finalized(&self) -> bool {
        self.justification.is_some()
    }

    pub fn hash(&self) -> Blake2bHash {
        self.header.hash()
    }
}

impl MacroJustification {
    pub fn verify(&self, block_hash: Blake2bHash, threshold: u16) -> bool {
        self.pbft_proof.verify(block_hash, threshold)
    }
}

impl SerializeContent for MacroHeader {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> { Ok(self.serialize(writer)?) }
}

impl Hash for MacroHeader { }

impl fmt::Display for MacroBlock {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "[#{}, view {}, type Macro]",
               self.header.block_number,
               self.header.view_number)
    }
}
