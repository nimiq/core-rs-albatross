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

pub type SignedPbftPrepareMessage = signed::SignedMessage<PbftPrepareMessage>;


#[derive(Clone, Debug, Serialize, Deserialize, SerializeContent, PartialEq, Eq)]
pub struct PbftCommitMessage {
    pub block_hash: Blake2bHash, // 32 bytes
}

impl signed::Message for PbftCommitMessage {
    const PREFIX: u8 = signed::PREFIX_PBFT_COMMIT;
}

pub type SignedPbftCommitMessage = signed::SignedMessage<PbftCommitMessage>;


#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PbftProof {
    pub prepare: signed::AggregateProof<PbftPrepareMessage>,
    pub commit: signed::AggregateProof<PbftCommitMessage>,
}

impl PbftProof {
    pub fn new() -> Self {
        Self {
            prepare: signed::AggregateProof::new(),
            commit: signed::AggregateProof::new(),
        }
    }

    pub fn verify(&self, block_hash: Blake2bHash, threshold: usize) -> bool {
        // XXX if we manually hash the message prefix and the block hash, we don't need to clone
        let prepare = PbftPrepareMessage { block_hash: block_hash.clone() };
        let commit = PbftCommitMessage { block_hash: block_hash };

        self.prepare.verify(&prepare, None)
            && self.commit.verify(&commit, None)
            && (&self.prepare.signers & &self.commit.signers).len() > threshold
    }
}
