use std::io;
use std::fmt;

use crate::view_change::ViewChangeProof;
use beserial::{Deserialize, Serialize};
use hash::{Blake2bHash, Hash, SerializeContent};
use nimiq_bls::bls12_381::{PublicKey, Signature};
use crate::signed;
use crate::pbft::PbftPrepareMessage;
use crate::pbft::PbftCommitMessage;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MacroBlock {
    pub header: MacroHeader,
    pub justification: Option<MacroJustification>,
    pub extrinsics: Option<MacroExtrinsics>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MacroHeader {
    pub version: u16,

    // Digest
    #[beserial(len_type(u16))]
    pub validators: Vec<PublicKey>,
    pub block_number: u32,
    pub view_number: u16,
    pub parent_macro_hash: Blake2bHash,

    pub parent_hash: Blake2bHash,
    pub extrinsics_root: Blake2bHash,
    pub state_root: Blake2bHash,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MacroJustification {
    pub prepare: signed::AggregateProof<PbftPrepareMessage>,
    pub commit: signed::AggregateProof<PbftCommitMessage>,
    pub view_change_proof: Option<ViewChangeProof>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MacroExtrinsics {
    pub timestamp: u64,
    pub seed: Signature,
}

impl MacroBlock {
    pub fn verify(&self) -> bool {
        if self.header.block_number >= 1 && self.justification.is_none() {
            return false;
        }

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

impl MacroJustification {
    pub fn verify(&self, block_hash: Blake2bHash, threshold: usize) -> bool {
        /*// both have to be valid & >k sigs from prepare must be included in commit
        self.prepare.verify(&PbftPrepareMessage { block_hash: block_hash.clone() }, None)
            && self.commit.verify(&PbftCommitMessage { block_hash }, None)
            // TODO: Try to do this without cloning
            && (self.prepare.signers.clone() & self.commit.signers.clone()).count_ones() > threshold*/
        unimplemented!()
    }
}

impl SerializeContent for MacroHeader {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> { Ok(self.serialize(writer)?) }
}

impl Hash for MacroHeader { }

impl SerializeContent for MacroExtrinsics {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> { Ok(self.serialize(writer)?) }
}

impl Hash for MacroExtrinsics { }

impl fmt::Display for MacroBlock {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "[#{}, view {}, type Macro]",
               self.header.block_number,
               self.header.view_number)
    }
}
