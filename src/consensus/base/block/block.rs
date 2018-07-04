use beserial::{Deserialize, ReadBytesExt, Serialize};
use consensus::base::block::{BlockBody, BlockHeader, BlockInterlink};
use consensus::base::primitive::hash::{Blake2bHash, Hash};
use consensus::networks::NetworkId;
use std::io;

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

    pub fn verify(&self, timestamp_now: u32, network_id: NetworkId) -> bool {
        // Check that the timestamp is not too far into the future.
        if self.header.timestamp > timestamp_now + Block::TIMESTAMP_DRIFT_MAX {
            warn!("Invalid block - timestamp too far in the future");
            return false;
        }

        // Check that the header hash matches the difficulty.
        if !self.header.verify_proof_of_work() {
            warn!("Invalid block - PoW verification failed");
            return false;
        }

        // Check that the maximum block size is not exceeded.
        if self.serialized_size() > Block::MAX_SIZE {
            warn!("Invalid block - max block size exceeded");
            return false;
        }

        // Verify that the interlink is valid.
        if !self.verify_interlink(network_id) {
            return false;
        }

        // Verify the body if it is present.
        if let Option::Some(ref body) = self.body {
            if !self.verify_body(body, network_id) {
                return false;
            }
        }

        // Everything checks out.
        return true;
    }

    fn verify_interlink(&self, network_id: NetworkId) -> bool {
        // Skip check for genesis block due to the cyclic dependency (since the interlink hash contains the genesis block hash).
        if self.header.height == 1 && self.header.interlink_hash == Blake2bHash::from([0u8; Blake2bHash::SIZE]) {
            return true;
        }

        // Check that the interlink_hash given in the header matches the actual interlink_hash.
        if self.header.interlink_hash != self.interlink.hash(network_id) {
            warn!("Invalid block - interlink hash mismatch");
            return false;
        }

        // Everything checks out.
        return true;
    }

    fn verify_body(&self, body: &BlockBody, network_id: NetworkId) -> bool {
        // Check that the body is valid.
        if !body.verify(network_id) {
            return false;
        }

        // Check that bodyHash given in the header matches the actual body hash.
        if self.header.body_hash != body.hash() {
            warn!("Invalid block - body hash mismatch");
            return false;
        }

        // Everything checks out.
        return true;
    }
}
