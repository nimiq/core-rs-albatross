use beserial::{Deserialize, Serialize};

use hash::{Blake2bHash, SerializeContent};
use super::signed;


#[derive(Clone, Debug, Serialize, Deserialize, SerializeContent, PartialEq, Eq)]
pub struct PbftPrepareMessage {
    pub block_hash: Blake2bHash, // 32 bytes
}


impl signed::Message for PbftPrepareMessage {
    const PREFIX: u8 = signed::PREFIX_PBFT_PREPARE;
}

#[derive(Clone, Debug, Serialize, Deserialize, SerializeContent, PartialEq, Eq)]
pub struct PbftCommitMessage {
    pub block_hash: Blake2bHash, // 32 bytes
}

impl signed::Message for PbftCommitMessage {
    const PREFIX: u8 = signed::PREFIX_PBFT_COMMIT;
}


#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PbftJustification {
    prepare: signed::AggregateProof<PbftPrepareMessage>,
    commit: signed::AggregateProof<PbftCommitMessage>
}

impl PbftJustification {
    pub fn verify(&self, block_hash: Blake2bHash, threshold: usize) -> bool {
        // both have to be valid & >k sigs from prepare must be included in commit
        self.prepare.verify(&PbftPrepareMessage { block_hash: block_hash.clone() }, None)
            && self.commit.verify(&PbftCommitMessage { block_hash }, None)
            // TODO: Try to do this without cloning
            && (self.prepare.signers.clone() & self.commit.signers.clone()).count_ones() > threshold
    }
}
