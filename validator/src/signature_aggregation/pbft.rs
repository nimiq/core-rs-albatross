use std::sync::Arc;
use std::collections::HashMap;
use std::fmt;

use parking_lot::RwLock;

use primitives::validators::Validators;
use block_albatross::{PbftPrepareMessage, PbftCommitMessage};
use messages::Message;
use hash::Blake2bHash;
use network::Peer;
use collections::bitset::BitSet;

use handel::update::LevelUpdate;
use handel::evaluator::{Evaluator, WeightedVote};
use handel::protocol::Protocol;
use handel::store::{SignatureStore, ReplaceStore};
use handel::aggregation::Aggregation;
use handel::config::Config;
use handel::partitioner::BinomialPartitioner;
use handel::verifier::MultithreadedVerifier;
use handel::multisig::{Signature, MultiSignature};

use super::voting::{VotingProtocol, Tag, VotingEvaluator, VotingSender, VoteAggregation, ValidatorRegistry};



impl Tag for PbftPrepareMessage {
    fn create_level_update_message(&self, update: LevelUpdate) -> Message {
        Message::HandelPbftPrepare(Box::new(update.with_tag(self.clone())))
    }
}

impl Tag for PbftCommitMessage {
    fn create_level_update_message(&self, update: LevelUpdate) -> Message {
        Message::HandelPbftCommit(Box::new(update.with_tag(self.clone())))
    }
}


/// The protocol and aggregation for prepare is just a vote.
pub type PbftPrepareProtocol = VotingProtocol<PbftPrepareMessage>;


/// The commit evalutor has a inner voting evaluator, which it uses for the implementation of `evaluate`.
/// The `is_final` must be adapted to take into account the signers of the prepare aggregation
pub struct PbftCommitEvaluator {
    /// The inner evaluator for the commits - ignoring the `is_final`.
    commit_evaluator: VotingEvaluator,

    /// The prepare aggregation. We need this to access the evaluator and signature store of the
    /// prepare phase.
    prepare_aggregation: Arc<Aggregation<PbftPrepareProtocol>>,
}

impl Evaluator for PbftCommitEvaluator {
    fn evaluate(&self, signature: &Signature, level: usize) -> usize {
        self.commit_evaluator.evaluate(signature, level)
    }

    fn is_final(&self, signature: &Signature) -> bool {
        signature.signers_bitset().intersection_size(&self.prepare_signers())
            >= self.commit_evaluator.threshold
    }
}

impl PbftCommitEvaluator {
    /// Returns the `BitSet` of signers of the prepare phase.
    fn prepare_signers(&self) -> BitSet {
        let store = self.prepare_aggregation.protocol.store();
        let store = store.read();
        // FIXME: I think this doesn't return the best combined signature.
        store.best(store.best_level())
            .map(|signature| signature.signers.clone())
            .unwrap_or_default()
    }
}


/// The generic protocol implementation for validator voting
pub struct PbftCommitProtocol {
    pub tag: PbftCommitMessage,

    store: Arc<RwLock<ReplaceStore<BinomialPartitioner>>>,

    /// The evaluator being used. This either just counts votes
    evaluator: Arc<PbftCommitEvaluator>,

    sender: Arc<VotingSender<PbftCommitMessage>>,

    prepare_aggregation: Arc<Aggregation<PbftPrepareProtocol>>,
}

impl Protocol for PbftCommitProtocol {
    type Registry = ValidatorRegistry;
    type Verifier = MultithreadedVerifier<ValidatorRegistry>;
    type Store = ReplaceStore<BinomialPartitioner>;
    type Evaluator = PbftCommitEvaluator;
    type Partitioner = BinomialPartitioner;
    type Sender = VotingSender<PbftCommitMessage>;

    fn registry(&self) -> Arc<Self::Registry> {
        Arc::clone(&self.prepare_aggregation.protocol.registry())
    }

    fn verifier(&self) -> Arc<Self::Verifier> {
        Arc::clone(&self.prepare_aggregation.protocol.verifier())
    }

    fn store(&self) -> Arc<RwLock<Self::Store>> {
        Arc::clone(&self.store)
    }

    fn evaluator(&self) -> Arc<Self::Evaluator> {
        Arc::clone(&self.evaluator)
    }

    fn partitioner(&self) -> Arc<Self::Partitioner> {
        Arc::clone(&self.prepare_aggregation.protocol.partitioner())
    }

    fn sender(&self) -> Arc<Self::Sender> {
        Arc::clone(&self.sender)
    }

    fn node_id(&self) -> usize {
        self.prepare_aggregation.protocol.node_id
    }
}

impl PbftCommitProtocol {
    pub fn new(prepare_aggregation: Arc<Aggregation<PbftPrepareProtocol>>) -> Self {
        let prepare_protocol = &prepare_aggregation.protocol;

        let tag = PbftCommitMessage::from(prepare_protocol.tag.block_hash.clone());
        let registry = Arc::clone(&prepare_protocol.registry());
        let partitioner = Arc::clone(&prepare_protocol.partitioner());
        let store = Arc::new(RwLock::new(ReplaceStore::new(Arc::clone(&partitioner))));
        let evaluator = Arc::new(PbftCommitEvaluator {
            commit_evaluator: VotingEvaluator::new(
                Arc::clone(&store),
                Arc::clone(&registry),
                Arc::clone(&partitioner),
                prepare_protocol.evaluator().threshold
            ),
            prepare_aggregation: Arc::clone(&prepare_aggregation),
        });
        // TODO: If we Arc the peer list, we don't need two copies. Or decouple the peer list from
        // the sender.
        let peers = prepare_protocol.sender().peers.clone();
        let sender = Arc::new(VotingSender::new(tag.clone(), peers));

        Self {
            tag,
            store,
            evaluator,
            sender,
            prepare_aggregation,
        }
    }
}

impl fmt::Debug for PbftCommitProtocol {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "PbftCommitProtocol {{ node_id: {}, {:?} }}", self.node_id(), self.tag)
    }
}



struct PbftAggregation {
    prepare_aggregation: Arc<Aggregation<PbftPrepareProtocol>>,
    commit_aggregation: Arc<Aggregation<PbftCommitProtocol>>,
}


impl PbftAggregation {
    pub fn new(proposal_hash: Blake2bHash, node_id: usize, validators: Validators, peers: HashMap<usize, Arc<Peer>>, config: Option<Config>) -> Self {
        let config = config.unwrap_or_default();

        let prepare_protocol = PbftPrepareProtocol::new(
            PbftPrepareMessage::from(proposal_hash.clone()),
            node_id,
            validators,
            &config,
            peers,

        );
        let prepare_aggregation = Aggregation::new(prepare_protocol, config.clone());

        let commit_protocol = PbftCommitProtocol::new(Arc::clone(&prepare_aggregation));
        let commit_aggregation = Aggregation::new(commit_protocol, config);

        Self {
            prepare_aggregation,
            commit_aggregation,
        }
    }
}