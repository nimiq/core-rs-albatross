use beserial::{Deserialize, Serialize};
use consensus::base::primitive::hash::{Blake2bHash, Hash, Hasher, SerializeContent};
use std::io;

#[derive(Default, Clone, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
pub struct TargetCompact(u32);

#[derive(Default, Clone, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
pub struct BlockHeader {
    pub version: u16,
    pub prev_hash: Blake2bHash,
    pub interlink_hash: Blake2bHash,
    pub body_hash: Blake2bHash,
    pub accounts_hash: Blake2bHash,
    pub n_bits: TargetCompact,
    pub height: u32,
    pub timestamp: u32,
    pub nonce: u32,
}

impl From<TargetCompact> for u32 {
    fn from(t: TargetCompact) -> Self {
        return t.0;
    }
}

impl From<u32> for TargetCompact {
    fn from(u: u32) -> Self {
        return TargetCompact(u);
    }
}

impl SerializeContent for BlockHeader {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> { self.serialize(writer) }
}

impl Hash for BlockHeader {}
