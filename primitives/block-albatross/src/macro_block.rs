use std::fmt;

use crate::pbft::PbftJustification;
use hash::{Hash, Blake2bHash, SerializeContent};
use beserial::{Deserialize, Serialize};
use nimiq_keys::{Signature, PublicKey};
use std::io;
use crate::view_change::ViewChangeProof;

#[derive(Clone, Debug, Ord, PartialOrd, Eq, PartialEq, Serialize, Deserialize)]
pub struct MacroDigest {
    #[beserial(len_type(u16))]
    pub validators: Vec<PublicKey>,
    pub parent_macro_hash: Blake2bHash,
    pub block_number: u32,
    pub view_number: u16,
}

impl SerializeContent for MacroDigest {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> { Ok(self.serialize(writer)?) }
}

impl Hash for MacroDigest { }

#[derive(Clone, Debug, Ord, PartialOrd, Eq, PartialEq, Serialize, Deserialize)]
pub struct MacroHeader {
    pub parent_hash: Blake2bHash,
    pub digest: MacroDigest,
    pub extrinsics_root: Blake2bHash,
    pub state_root: Blake2bHash,
}

impl SerializeContent for MacroHeader {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> { Ok(self.serialize(writer)?) }
}

impl Hash for MacroHeader { }

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MacroExtrinsics {
    pub timestamp: u64,
    pub seed: u64,
    pub producer_public_key: PublicKey,
    pub seed_signature: Signature,
    pub view_change_proof: Option<ViewChangeProof>,
}

impl SerializeContent for MacroExtrinsics {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> { Ok(self.serialize(writer)?) }
}

impl Hash for MacroExtrinsics { }

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MacroBlock {
    pub header: MacroHeader,
    pub extrinsics: MacroExtrinsics,
    pub justification: Option<PbftJustification>,
}

impl MacroBlock {
    pub fn verify(&self) -> bool {
        if let Some(justification) = &self.justification {
            if !justification.verify(self.hash(), unimplemented!("Signature Threshold")) {
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

impl fmt::Display for MacroBlock {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "[#{}, view {}, type Macro]",
               self.header.digest.block_number,
               self.header.digest.view_number)
    }
}
