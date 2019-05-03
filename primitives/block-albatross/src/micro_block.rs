use std::fmt;

use std::io;

use crate::slash::SlashInherent;
use beserial::{Deserialize, Serialize};
use hash::{Hash, Blake2bHash, SerializeContent};
use keys::{Signature, PublicKey};
use transaction::Transaction;
use crate::view_change::ViewChangeProof;

#[derive(Clone, Debug, Ord, PartialOrd, Eq, PartialEq, Serialize, Deserialize)]
pub struct MicroDigest {
    pub validator: PublicKey,
    pub block_number: u32,
    pub view_number: u16,
}

#[derive(Clone, Debug, Ord, PartialOrd, Eq, PartialEq, Serialize, Deserialize)]
pub struct MicroHeader {
    pub parent_hash: Blake2bHash,
    pub digest: MicroDigest,
    pub extrinsics_root: Blake2bHash,
    pub state_root: Blake2bHash,
}

impl SerializeContent for MicroHeader {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> { Ok(self.serialize(writer)?) }
}

impl Hash for MicroHeader { }

impl fmt::Display for MicroHeader {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "[#{} view {}, type Micro]", self.digest.block_number, self.digest.view_number)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MicroExtrinsics {
    pub timestamp: u64,
    pub seed: u64,
    pub seed_signature: Signature,
    pub view_change_proof: Option<ViewChangeProof>,
    #[beserial(len_type(u16))]
    pub slash_inherents: Vec<SlashInherent>,
    #[beserial(len_type(u16))]
    pub transactions: Vec<Transaction>,
}

impl SerializeContent for MicroExtrinsics {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> { Ok(self.serialize(writer)?) }
}

impl Hash for MicroExtrinsics { }

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MicroBlock {
    pub header: MicroHeader,
    pub extrinsics: MicroExtrinsics,
    pub justification: Signature,
}
