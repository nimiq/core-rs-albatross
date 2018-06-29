use beserial::{Deserialize, Serialize};
use consensus::base::primitive::hash::{Argon2dHasher, Blake2bHash, Blake2bHasher, Hash, Hasher};

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

impl Hash<Blake2bHasher> for BlockHeader {
    fn hash(&self, state: &mut Blake2bHasher) {
        state.write(&self.serialize_to_vec()[..]);
    }
}

impl Hash<Argon2dHasher> for BlockHeader {
    fn hash(&self, state: &mut Argon2dHasher) {
        state.write(&self.serialize_to_vec()[..]);
    }
}
