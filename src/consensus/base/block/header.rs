use beserial::{Deserialize, Serialize};
use consensus::base::block::{Target, TargetCompact};
use consensus::base::primitive::hash::{Argon2dHash, Blake2bHash, Hash, SerializeContent};
use std::io;

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

impl SerializeContent for BlockHeader {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> { self.serialize(writer) }
}

impl Hash for BlockHeader {}

impl BlockHeader {
    pub(super) fn verify_proof_of_work(&self) -> bool {
        let pow: Argon2dHash = self.hash();
        let target: Target = self.n_bits.into();
        return target.is_reached_by(&pow);
    }
}
