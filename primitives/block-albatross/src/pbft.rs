use beserial::{Deserialize, Serialize};
use bls::PublicKey;
use hash::{Blake2bHash, SerializeContent};
use hash_derive::SerializeContent;
use primitives::slot::ValidatorSlots;

use crate::ViewChangeProof;

use super::signed;
use super::signed::{
    votes_for_signers, AggregateProof, AggregateProofBuilder, AggregateProofError, Message,
    SignedMessage,
};
use super::MacroHeader;

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

impl From<Blake2bHash> for PbftPrepareMessage {
    fn from(block_hash: Blake2bHash) -> Self {
        PbftPrepareMessage { block_hash }
    }
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

impl From<Blake2bHash> for PbftCommitMessage {
    fn from(block_hash: Blake2bHash) -> Self {
        PbftCommitMessage { block_hash }
    }
}

pub type SignedPbftCommitMessage = SignedMessage<PbftCommitMessage>;

/// A pBFT proof - which is a composition of the prepare proof and the commit proof
/// It verifies both proofs individually and then checks that at least `threshold` validators who
/// signed the prepare also signed the commit.
///
/// DEPRECATED: We don't use this anymore
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
    pub fn verify_prepare(
        &self,
        block_hash: Blake2bHash,
        threshold: u16,
    ) -> Result<(), AggregateProofError> {
        let prepare = PbftPrepareMessage { block_hash };
        self.prepare.verify(&prepare, threshold)
    }

    /// Verify that we have enough valid commit signatures that also signed the prepare
    /// TODO: Duplicate code (in PbftProof)
    pub fn verify(
        &self,
        block_hash: Blake2bHash,
        validators: &ValidatorSlots,
        threshold: u16,
    ) -> Result<(), AggregateProofError> {
        // XXX if we manually hash the message prefix and the block hash, we don't need to clone
        let prepare = PbftPrepareMessage {
            block_hash: block_hash.clone(),
        };
        let commit = PbftCommitMessage { block_hash };

        self.prepare.verify(&prepare, threshold)?;
        self.commit.verify(&commit, threshold)?;

        // sum up votes of signers that signed prepare and commit
        let signers = &self.prepare.signers & &self.commit.signers;
        let votes = votes_for_signers(validators, &signers)?;
        trace!("votes on prepare and commit: {}", votes);

        if votes < threshold {
            return Err(AggregateProofError::InsufficientSigners(votes, threshold));
        }
        Ok(())
    }

    pub fn add_prepare_signature(
        &mut self,
        public_key: &PublicKey,
        num_slots: u16,
        prepare: &SignedPbftPrepareMessage,
    ) -> bool {
        self.prepare.add_signature(public_key, num_slots, prepare)
    }

    pub fn add_commit_signature(
        &mut self,
        public_key: &PublicKey,
        num_slots: u16,
        commit: &SignedPbftCommitMessage,
    ) -> bool {
        self.commit.add_signature(public_key, num_slots, commit)
    }

    pub fn clear(&mut self) {
        self.prepare.clear();
        self.commit.clear();
    }

    pub fn build(self) -> PbftProof {
        PbftProof {
            prepare: self.prepare.build(),
            commit: self.commit.build(),
        }
    }
}

impl Default for PbftProofBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PbftProof {
    pub prepare: AggregateProof<PbftPrepareMessage>,
    pub commit: AggregateProof<PbftCommitMessage>,
}

impl PbftProof {
    pub fn votes(&self, validators: &ValidatorSlots) -> Result<u16, AggregateProofError> {
        let signers = &self.prepare.signers & &self.commit.signers;
        votes_for_signers(validators, &signers)
    }

    pub fn verify(
        &self,
        block_hash: Blake2bHash,
        validators: &ValidatorSlots,
        threshold: u16,
    ) -> Result<(), AggregateProofError> {
        // XXX if we manually hash the message prefix and the block hash, we don't need to clone
        let prepare = PbftPrepareMessage {
            block_hash: block_hash.clone(),
        };
        let commit = PbftCommitMessage { block_hash };

        self.prepare
            .verify(&prepare, validators, threshold)
            .map_err(|e| {
                trace!("prepare verify failed");
                e
            })?;
        self.commit
            .verify(&commit, validators, threshold)
            .map_err(|e| {
                trace!("commit verify failed");
                e
            })?;

        let votes = self.votes(validators)?;
        trace!("votes on prepare and commit: {}", votes);

        if votes < threshold {
            return Err(AggregateProofError::InsufficientSigners(votes, threshold));
        }
        Ok(())
    }
}
