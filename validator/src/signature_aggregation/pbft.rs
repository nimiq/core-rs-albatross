use std::sync::Arc;

use parking_lot::RwLock;

use block_albatross::{
    IndividualSignature, MultiSignature, PbftCommitMessage, PbftPrepareMessage,
    SignedPbftCommitMessage, SignedPbftPrepareMessage,
};
use hash::Blake2bHash;
use messages::Message;

use handel::aggregation::Aggregation;
use handel::config::Config;
use handel::protocol::Protocol;
use handel::store::ContributionStore;
use handel::update::{LevelUpdate, LevelUpdateMessage};

use super::voting::{Tag, VotingProtocol};
use crate::pool::ValidatorPool;

use super::voting::{Tag, ValidatorRegistry, VotingEvaluator, VotingProtocol, VotingSender};

impl Tag for PbftPrepareMessage {
    fn create_level_update_message(&self, update: LevelUpdate<MultiSignature>) -> Message {
        Message::PbftPrepare(Box::new(update.with_tag(self.clone())))
    }
}

impl Tag for PbftCommitMessage {
    fn create_level_update_message(&self, update: LevelUpdate<MultiSignature>) -> Message {
        Message::PbftCommit(Box::new(update.with_tag(self.clone())))
    }
}

/// The protocol and aggregation for prepare is just a vote.
pub type PbftPrepareProtocol = VotingProtocol<PbftPrepareMessage>;
/// Same holds true for commit
pub type PbftCommitProtocol = VotingProtocol<PbftCommitMessage>;

pub struct PbftAggregation {
    pub prepare_aggregation: Arc<Aggregation<PbftPrepareProtocol>>,
    pub commit_aggregation: Arc<Aggregation<PbftCommitProtocol>>,
}

impl PbftAggregation {
    pub fn new(
        proposal_hash: Blake2bHash,
        node_id: usize,
        validators: Arc<RwLock<ValidatorPool>>,
        config: Option<Config>,
    ) -> Self {
        let config = config.unwrap_or_default();

        // create prepare aggregation
        let prepare_protocol = PbftPrepareProtocol::new(
            PbftPrepareMessage::from(proposal_hash.clone()),
            node_id,
            validators.clone(),
        );
        let prepare_aggregation = Aggregation::new(prepare_protocol, config.clone());

        // create commit aggregation
        let commit_protocol = PbftCommitProtocol::new(
            PbftCommitMessage::from(proposal_hash.clone()),
            node_id,
            validators.clone(),
        );
        let commit_aggregation = Aggregation::new(commit_protocol, config);

        Self {
            prepare_aggregation,
            commit_aggregation,
        }
    }

    pub fn push_signed_prepare(&self, contribution: SignedPbftPrepareMessage) {
        // deconstruct signed view change
        let SignedPbftPrepareMessage {
            signature,
            message: tag,
            signer_idx: node_id,
        } = contribution;
        let node_id = node_id as usize;

        // panic if the contribution doesn't belong to this aggregation
        if self.prepare_aggregation.protocol.tag != tag {
            panic!(
                "Submitting prepare for {:?}, but aggregation is for {:?}",
                tag, self.prepare_aggregation.protocol.tag
            );
        }

        // panic if the contribution is from a different node
        if self.prepare_aggregation.protocol.node_id != node_id {
            panic!(
                "Submitting prepare for validator {}, but aggregation is running as validator {}",
                node_id,
                self.node_id()
            );
        }

        self.prepare_aggregation
            .push_contribution(IndividualSignature::new(signature, node_id).as_multisig());
    }

    pub fn push_signed_commit(&self, contribution: SignedPbftCommitMessage) {
        // deconstruct signed view change
        let SignedPbftCommitMessage {
            signature,
            message: tag,
            signer_idx: node_id,
        } = contribution;
        let node_id = node_id as usize;

        // panic if the contribution doesn't belong to this aggregation
        if self.commit_aggregation.protocol.tag != tag {
            panic!(
                "Submitting commit for {:?}, but aggregation is for {:?}",
                tag, self.commit_aggregation.protocol.tag
            );
        }

        // panic if the contribution is from a different node
        if self.prepare_aggregation.protocol.node_id != node_id {
            panic!(
                "Submitting commit for validator {}, but aggregation is running as validator {}",
                node_id,
                self.node_id()
            );
        }

        self.commit_aggregation
            .push_contribution(IndividualSignature::new(signature, node_id).as_multisig());
    }

    pub fn push_prepare_level_update(
        &self,
        level_update: LevelUpdateMessage<MultiSignature, PbftPrepareMessage>,
    ) {
        if level_update.tag != self.prepare_aggregation.protocol.tag {
            panic!(
                "Submitting level update for {:?}, but aggregation is for {:?}",
                level_update.tag, self.prepare_aggregation.protocol.tag
            );
        }
        self.prepare_aggregation.push_update(level_update.update);
    }

    pub fn push_commit_level_update(
        &self,
        level_update: LevelUpdateMessage<MultiSignature, PbftCommitMessage>,
    ) {
        if level_update.tag != self.commit_aggregation.protocol.tag {
            panic!(
                "Submitting level update for {:?}, but aggregation is for {:?}",
                level_update.tag, self.commit_aggregation.protocol.tag
            );
        }
        self.commit_aggregation.push_update(level_update.update);
    }

    pub fn proposal_hash(&self) -> &Blake2bHash {
        assert_eq!(
            self.prepare_aggregation.protocol.tag.block_hash,
            self.commit_aggregation.protocol.tag.block_hash
        );
        &self.prepare_aggregation.protocol.tag.block_hash
    }

    pub fn node_id(&self) -> usize {
        // NOTE: We don't need to assert that `node_id` is the same for prepare and commit, since
        // the commit aggregation just takes the value from the prepare aggregation when you call
        // the getter.
        self.prepare_aggregation.protocol.node_id
    }

    pub fn votes(&self) -> (usize, usize) {
        (
            self.prepare_aggregation.protocol.votes(),
            self.commit_aggregation.protocol.votes(),
        )
    }

    pub fn prepare_signature(&self) -> Option<MultiSignature> {
        let store = self.prepare_aggregation.protocol.store();
        let store = store.read();
        store.combined(store.best_level())
    }
}
