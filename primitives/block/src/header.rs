use std::io;

use beserial::{Deserialize, Serialize};
use hash::{Argon2dHash, Blake2bHash, Hash, SerializeContent};

use crate::{Target, TargetCompact};

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
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> { Ok(self.serialize(writer)?) }
}

// Different hash implementation than std
#[allow(clippy::derive_hash_xor_eq)]
impl Hash for BlockHeader {}

impl BlockHeader {
    pub const SIZE: usize = 146;

    pub fn verify_proof_of_work(&self) -> bool {
        let target: Target = self.n_bits.into();
        target.is_met_by(&self.pow())
    }

    pub fn pow(&self) -> Argon2dHash {
        self.hash()
    }

    pub fn is_immediate_successor_of(&self, prev_header: &BlockHeader) -> bool {
        // Check that the height is one higher than the previous height.
        if self.height != prev_header.height + 1 {
            return false;
        }

        // Check that the timestamp is greater or equal to the predecessor's timestamp.
        if self.timestamp < prev_header.timestamp {
            return false;
        }

        // Check that the hash of the predecessor block equals prevHash.
        let prev_hash: Blake2bHash = prev_header.hash();
        if self.prev_hash != prev_hash {
            return false;
        }

        // Everything checks out.
        true
    }

    pub fn timestamp_in_millis(&self) -> u64 {
        u64::from(self.timestamp) * 1000
    }
}
