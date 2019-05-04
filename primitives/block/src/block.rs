use beserial::{Deserialize, ReadBytesExt, Serialize, SerializingError};
use hash::{Argon2dHash, Blake2bHash, Hash};

use crate::{BlockBody, BlockError, BlockHeader, BlockInterlink, Target};
use primitives::networks::NetworkId;

#[derive(Default, Clone, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize)]
pub struct Block {
    pub header: BlockHeader,
    pub interlink: BlockInterlink,
    pub body: Option<BlockBody>,
}

impl Deserialize for Block {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let header: BlockHeader = Deserialize::deserialize(reader)?;
        let interlink = BlockInterlink::deserialize(reader, &header.prev_hash)?;
        Ok(Block {
            header,
            interlink,
            body: Deserialize::deserialize(reader)?,
        })
    }
}

impl Block {
    pub const VERSION: u16 = 1;
    pub const MAX_SIZE: usize = 100_000; // 100 kb
    const TIMESTAMP_DRIFT_MAX: u64 = 600 * 1000;

    pub fn verify(&self, timestamp_now: u64, network_id: NetworkId, genesis_hash: Blake2bHash) -> Result<(), BlockError> {
        // XXX Check that the block version is supported.
        if self.header.version != Block::VERSION {
            return Err(BlockError::UnsupportedVersion);
        }

        // Check that the timestamp is not too far into the future.
        // XXX Move this check to Blockchain?
        if self.header.timestamp_in_millis() > timestamp_now + Block::TIMESTAMP_DRIFT_MAX {
            return Err(BlockError::FromTheFuture);
        }

        // Check that the proof of work is valid.
        if !self.header.verify_proof_of_work() {
            return Err(BlockError::InvalidPoW);
        }

        // Check that the maximum block size is not exceeded.
        if self.serialized_size() > Block::MAX_SIZE {
            return Err(BlockError::SizeExceeded);
        }

        // Verify that the interlink is valid.
        self.verify_interlink(genesis_hash)?;

        // Verify the body if it is present.
        if let Option::Some(ref body) = self.body {
            self.verify_body(body, network_id)?;
        }

        // Everything fine.
        Ok(())
    }

    fn verify_interlink(&self, genesis_hash: Blake2bHash) -> Result<(), BlockError> {
        // Skip check for genesis block due to the cyclic dependency (since the interlink hash contains the genesis block hash).
        if self.header.height == 1 && self.header.interlink_hash == Blake2bHash::from([0u8; Blake2bHash::SIZE]) {
            return Ok(());
        }

        // Check that the interlink_hash given in the header matches the actual interlink_hash.
        if self.header.interlink_hash != self.interlink.hash(genesis_hash) {
            return Err(BlockError::InterlinkHashMismatch);
        }

        // Everything fine.
        Ok(())
    }

    fn verify_body(&self, body: &BlockBody, network_id: NetworkId) -> Result<(), BlockError> {
        // Check that the body is valid.
        body.verify(self.header.height, network_id)?;

        // Check that bodyHash given in the header matches the actual body hash.
        if self.header.body_hash != body.hash() {
            return Err(BlockError::BodyHashMismatch);
        }

        // Everything fine.
        Ok(())
    }

    pub fn is_immediate_successor_of(&self, predecessor: &Block) -> bool {
        // Check header.
        if !self.header.is_immediate_successor_of(&predecessor.header) {
            return false;
        }

        // Check that the interlink is correct.
        let interlink = predecessor.get_next_interlink(&self.header.n_bits.into());
        if self.interlink != interlink {
            return false;
        }

        // Everything fine.
        true
    }

    pub fn get_next_interlink(&self, next_target: &Target) -> BlockInterlink {
        let mut hashes: Vec<Blake2bHash> = vec![];
        let hash: Blake2bHash = self.header.hash();
        let pow: Argon2dHash = self.header.hash();

        // Compute how many times this blockHash should be included in the next interlink.
        let this_pow_depth = i16::from(Target::from(&pow).get_depth());
        let next_target_depth = i16::from(next_target.get_depth());
        let num_occurrences = (this_pow_depth - next_target_depth + 1).max(0);

        // Push this blockHash numOccurrences times onto the next interlink.
        for _ in 0..num_occurrences {
            hashes.push(hash.clone());
        }

        // Compute how many blocks to omit from the beginning of this interlink.
        let this_target_depth = i16::from(Target::from(self.header.n_bits).get_depth());
        let target_offset = next_target_depth - this_target_depth;
        let interlink_offset = (num_occurrences + target_offset) as usize;

        // Push the remaining hashes from this interlink.
        for i in interlink_offset..self.interlink.len() {
            hashes.push(self.interlink.hashes[i].clone());
        }

        BlockInterlink::new(hashes, &hash)
    }

    pub fn into_light(mut self) -> Block {
        self.body = None;
        self
    }

    pub fn is_light(&self) -> bool {
        self.body.is_none()
    }
}
