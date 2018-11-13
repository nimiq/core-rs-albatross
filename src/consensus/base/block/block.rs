use beserial::{Deserialize, ReadBytesExt, Serialize};
use consensus::base::block::{BlockBody, BlockHeader, BlockInterlink, Target, BlockError};
use consensus::base::primitive::hash::{Hash, Blake2bHash, Argon2dHash};
use consensus::networks::NetworkId;
use std::io;
use utils::db::{FromDatabaseValue, IntoDatabaseValue};

#[derive(Default, Clone, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize)]
pub struct Block {
    pub header: BlockHeader,
    pub interlink: BlockInterlink,
    pub body: Option<BlockBody>,
}

impl Deserialize for Block {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> io::Result<Self> {
        let header: BlockHeader = Deserialize::deserialize(reader)?;
        let interlink = BlockInterlink::deserialize(reader, &header.prev_hash)?;
        return Ok(Block {
            header,
            interlink,
            body: Deserialize::deserialize(reader)?,
        });
    }
}

impl Block {
    const TIMESTAMP_DRIFT_MAX: u32 = 600;
    const MAX_SIZE: usize = 100000; // 100 kb
    const VERSION: u16 = 1;

    pub fn verify(&self, timestamp_now: u32, network_id: NetworkId) -> Result<(), BlockError> {
        // XXX Check that the block version is supported.
        if self.header.version != Block::VERSION {
            return Err(BlockError::UnsupportedVersion);
        }

        // Check that the timestamp is not too far into the future.
        // XXX Move this check to Blockchain?
        if self.header.timestamp > timestamp_now + Block::TIMESTAMP_DRIFT_MAX {
            return Err(BlockError::FromTheFuture);
        }

        // Check that the header hash matches the difficulty.
        if !self.header.verify_proof_of_work() {
            return Err(BlockError::InvalidPoW);
        }

        // Check that the maximum block size is not exceeded.
        if self.serialized_size() > Block::MAX_SIZE {
            return Err(BlockError::SizeExceeded);
        }

        // Verify that the interlink is valid.
        self.verify_interlink(network_id)?;

        // Verify the body if it is present.
        if let Option::Some(ref body) = self.body {
            self.verify_body(body, network_id)?;
        }

        // Everything checks out.
        return Ok(());
    }

    fn verify_interlink(&self, network_id: NetworkId) -> Result<(), BlockError> {
        // Skip check for genesis block due to the cyclic dependency (since the interlink hash contains the genesis block hash).
        if self.header.height == 1 && self.header.interlink_hash == Blake2bHash::from([0u8; Blake2bHash::SIZE]) {
            return Ok(());
        }

        // Check that the interlink_hash given in the header matches the actual interlink_hash.
        if self.header.interlink_hash != self.interlink.hash(network_id) {
            return Err(BlockError::InterlinkHashMismatch);
        }

        // Everything checks out.
        return Ok(());
    }

    fn verify_body(&self, body: &BlockBody, network_id: NetworkId) -> Result<(), BlockError> {
        // Check that the body is valid.
        body.verify(self.header.height, network_id)?;

        // Check that bodyHash given in the header matches the actual body hash.
        if self.header.body_hash != body.hash() {
            return Err(BlockError::BodyHashMismatch);
        }

        // Everything checks out.
        return Ok(());
    }

    pub fn is_immediate_successor_of(&self, predecessor: &Block) -> bool {
        // Check header.
        if !self.header.is_immediate_successor_of(&predecessor.header) {
            return false;
        }

        // Check that the interlink is correct.
        let interlink = predecessor.get_next_interlink(self.header.n_bits.into());
        if self.interlink != interlink {
            return false;
        }

        // Everything checks out.
        return true;
    }

    pub fn get_next_interlink(&self, next_target: Target) -> BlockInterlink {
        let mut hashes: Vec<Blake2bHash> = vec![];
        let hash: Blake2bHash = self.header.hash();
        let pow: Argon2dHash = self.header.hash();

        // Compute how many times this blockHash should be included in the next interlink.
        let this_pow_depth = Target::from(&pow).get_depth() as i16;
        let next_target_depth = next_target.get_depth() as i16;
        let num_occurrences = (this_pow_depth - next_target_depth + 1).max(0);

        // Push this blockHash numOccurrences times onto the next interlink.
        for i in 0..num_occurrences {
            hashes.push(hash.clone());
        }

        // Compute how many blocks to omit from the beginning of this interlink.
        let this_target_depth = Target::from(self.header.n_bits).get_depth() as i16;
        let target_offset = next_target_depth - this_target_depth;
        let interlink_offset = (num_occurrences + target_offset) as usize;

        // Push the remaining hashes from this interlink.
        for i in interlink_offset..self.interlink.len() {
            hashes.push(self.interlink.hashes[i].clone());
        }

        return BlockInterlink::new(hashes, &hash);
    }
}

impl IntoDatabaseValue for Block {
    fn database_byte_size(&self) -> usize {
        return self.serialized_size();
    }

    fn copy_into_database(&self, mut bytes: &mut [u8]) {
        Serialize::serialize(&self, &mut bytes).unwrap();
    }
}

impl FromDatabaseValue for Block {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self> where Self: Sized {
        let mut cursor = io::Cursor::new(bytes);
        return Deserialize::deserialize(&mut cursor);
    }
}

