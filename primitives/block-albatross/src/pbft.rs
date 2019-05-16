use beserial::{Deserialize, Serialize};
use bls::bls12_381::PublicKey;
use hash::{Blake2bHash, SerializeContent};
use primitives::validators::Validators;

use crate::ViewChangeProof;

use super::MacroHeader;
use super::signed;
use super::signed::{AggregateProof, AggregateProofBuilder, AggregateProofError, Message, SignedMessage};

/// A macro block proposed by the pBFT-leader.
#[derive(Clone, Debug, Serialize, Deserialize, SerializeContent, PartialEq, Eq)]
pub struct PbftProposal {
    pub header: MacroHeader,
    pub view_change: Option<ViewChangeProof>,
}
pub type SignedPbftProposal = SignedMessage<PbftProposal>;

impl Message for PbftProposal {
    const PREFIX: u8 = signed::PREFIX_PBFT_PROPOSAL;
}


/// a pBFT prepare message - references the proposed macro block by hash
#[derive(Clone, Debug, Serialize, Deserialize, SerializeContent, PartialEq, Eq)]
pub struct PbftPrepareMessage {
    pub block_hash: Blake2bHash, // 32 bytes
}

impl Message for PbftPrepareMessage {
    const PREFIX: u8 = signed::PREFIX_PBFT_PREPARE;
}

pub type SignedPbftPrepareMessage = SignedMessage<PbftPrepareMessage>;


/// A pBFT commit message - references the proposed macro block by hash
#[derive(Clone, Debug, Serialize, Deserialize, SerializeContent, PartialEq, Eq)]
pub struct PbftCommitMessage {
    pub block_hash: Blake2bHash, // 32 bytes
}

impl Message for PbftCommitMessage {
    const PREFIX: u8 = signed::PREFIX_PBFT_COMMIT;
}

pub type SignedPbftCommitMessage = SignedMessage<PbftCommitMessage>;


/// A pBFT proof - which is a composition of the prepare proof and the commit proof
/// It verifies both proofs individually and then checks that at least `threshold` validators who
/// signed the prepare also signed the commit.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PbftProofBuilder {
    pub prepare: AggregateProofBuilder<PbftPrepareMessage>,
    pub commit: AggregateProofBuilder<PbftCommitMessage>,
}

impl PbftProofBuilder {
    pub fn new() -> Self {
        Self {
            prepare: AggregateProofBuilder::new(),
            commit: AggregateProofBuilder::new(),
        }
    }

    /// Verify if we have enough valid signatures in the prepare phase
    pub fn verify_prepare(&self, block_hash: Blake2bHash, threshold: u16) -> Result<(), AggregateProofError> {
        let prepare = PbftPrepareMessage{ block_hash };
        self.prepare.verify(&prepare, threshold)
    }

    /// Verify that we have enough valid commit signatures that also signed the prepare
    pub fn verify(&self, block_hash: Blake2bHash, threshold: u16) -> Result<(), AggregateProofError> {
        // XXX if we manually hash the message prefix and the block hash, we don't need to clone
        let prepare = PbftPrepareMessage { block_hash: block_hash.clone() };
        let commit = PbftCommitMessage { block_hash };

        self.prepare.verify(&prepare, threshold)?;
        self.commit.verify(&commit, threshold)?;
        if ((&self.prepare.signers & &self.commit.signers).len() as u16) < threshold {
            return Err(AggregateProofError::InsufficientSigners)
        }
        Ok(())
    }

    pub fn add_prepare_signature(&mut self, public_key: &PublicKey, num_slots: u16, prepare: &SignedPbftPrepareMessage) -> bool {
        self.prepare.add_signature(public_key, num_slots, prepare)
    }

    pub fn add_commit_signature(&mut self, public_key: &PublicKey, num_slots: u16, commit: &SignedPbftCommitMessage) -> bool {
        self.commit.add_signature(public_key, num_slots, commit)
    }

    pub fn clear(&mut self) {
        self.prepare.clear();
        self.commit.clear();
    }

    pub fn build(self) -> PbftProof {
        PbftProof {
            prepare: self.prepare.build(),
            commit: self.commit.build()
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PbftProof {
    pub prepare: AggregateProof<PbftPrepareMessage>,
    pub commit: AggregateProof<PbftCommitMessage>,
}

impl PbftProof {
    pub fn verify(&self, block_hash: Blake2bHash, validators: &Validators, threshold: u16) -> Result<(), AggregateProofError> {
        // XXX if we manually hash the message prefix and the block hash, we don't need to clone
        let prepare = PbftPrepareMessage { block_hash: block_hash.clone() };
        let commit = PbftCommitMessage { block_hash };

        self.prepare.verify(&prepare, validators, threshold)?;
        self.commit.verify(&commit, validators, threshold)?;

        // sum up votes of signers that signed prepare and commit
        let votes: u16 = (&self.prepare.signers & &self.commit.signers).iter().map(|s| {
            validators.get(s).map(|v| v.num_slots).unwrap_or(0)
        }).sum();

        if votes < threshold {
            return Err(AggregateProofError::InsufficientSigners)
        }
        Ok(())
    }
}
