use std::sync::Arc;
use std::io::Error as IoError;
use std::io::ErrorKind;

use parking_lot::RwLock;

use primitives::validators::Validators;
use primitives::policy::TWO_THIRD_SLOTS;
use block_albatross::ViewChange;
use network::Peer;
use messages::Message;

use handel::protocol::Protocol;
use handel::multisig::{MultiSignature, IndividualSignature, Signature};
use handel::identity::{IdentityRegistry, WeightRegistry};
use handel::verifier::MultithreadedVerifier;
use handel::timeout::LinearTimeout;
use handel::config::Config;
use handel::store::ReplaceStore;
use handel::partitioner::BinomialPartitioner;
use handel::evaluator::WeightedVote;
use handel::message::AggregationMessage;

use super::validators::ValidatorRegistry;



pub type ViewChangeEvaluator = WeightedVote<ReplaceStore<BinomialPartitioner>, ValidatorRegistry, BinomialPartitioner>;


pub struct ViewChangeProtocol {
    pub view_change: ViewChange,

    registry: Arc<ValidatorRegistry>,
    verifier: Arc<MultithreadedVerifier<ValidatorRegistry>>,
    timeouts: Arc<LinearTimeout>,
    partitioner: Arc<BinomialPartitioner>,
    store: Arc<RwLock<ReplaceStore<BinomialPartitioner>>>,
    evaluator: Arc<ViewChangeEvaluator>,
}

impl ViewChangeProtocol {
    pub fn new(view_change: ViewChange, node_id: usize, validators: Validators, config: Config) -> Self {
        let num_validators = validators.len();

        let registry = Arc::new(ValidatorRegistry::new(validators));
        let verifier = Arc::new(MultithreadedVerifier::new(
            config.message_hash,
            Arc::clone(&registry),
            None
        ));
        let timeouts = Arc::new(LinearTimeout::new(config.timeout));
        let partitioner = Arc::new(BinomialPartitioner::new(
            node_id,
            num_validators
        ));
        let store = Arc::new(RwLock::new(ReplaceStore::new(Arc::clone(&partitioner))));
        let evaluator = Arc::new(WeightedVote::new(
            Arc::clone(&store),
            Arc::clone(&registry),
            Arc::clone(&partitioner),
            TWO_THIRD_SLOTS as usize,
        ));

        Self {
            view_change,
            registry,
            verifier,
            timeouts,
            partitioner,
            store,
            evaluator,
        }
    }
}

impl Protocol for ViewChangeProtocol {
    type Registry = ValidatorRegistry;
    type Verifier = MultithreadedVerifier<ValidatorRegistry>;
    type Timeouts = LinearTimeout;
    type Store = ReplaceStore<BinomialPartitioner>;
    type Evaluator = ViewChangeEvaluator;
    type Partitioner = BinomialPartitioner;

    fn registry(&self) -> Arc<Self::Registry> {
        Arc::clone(&self.registry)
    }

    fn verifier(&self) -> Arc<Self::Verifier> {
        Arc::clone(&self.verifier)
    }

    fn timeouts(&self) -> Arc<Self::Timeouts> {
        Arc::clone(&self.timeouts)
    }

    fn store(&self) -> Arc<RwLock<Self::Store>> {
        Arc::clone(&self.store)
    }

    fn evaluator(&self) -> Arc<Self::Evaluator> {
        Arc::clone(&self.evaluator)
    }

    fn partitioner(&self) -> Arc<Self::Partitioner> {
        Arc::clone(&self.partitioner)
    }

    fn send_to(&self, peer_id: usize, multisig: MultiSignature, individual: Option<IndividualSignature>, level: usize) -> Result<(), IoError> {
        let peer: Peer = unimplemented!();

        let message = AggregationMessage::new(self.view_change.clone(), multisig, individual, level);
        peer.channel.send(Message::HandelViewChange(Box::new(message)))
            .map_err(|e| IoError::new(ErrorKind::Other, e))
    }
}
