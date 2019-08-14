use std::sync::Arc;
use std::io::Error as IoError;
use std::io::ErrorKind;
use std::time::Duration;
use std::collections::HashMap;
use std::fmt;

use parking_lot::RwLock;

use primitives::validators::Validators;
use primitives::policy::TWO_THIRD_SLOTS;
use block_albatross::{ViewChange, SignedViewChange};
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
use handel::update::{LevelUpdate, LevelUpdateMessage};
use handel::aggregation::Aggregation;
use handel::store::SignatureStore;

use super::validators::ValidatorRegistry;
use block_albatross::signed::Message as SignedMessage;
use beserial::Serialize;


pub type ViewChangeEvaluator = WeightedVote<ReplaceStore<BinomialPartitioner>, ValidatorRegistry, BinomialPartitioner>;


pub struct ViewChangeProtocol {
    pub view_change: ViewChange,
    pub node_id: usize,

    registry: Arc<ValidatorRegistry>,
    verifier: Arc<MultithreadedVerifier<ValidatorRegistry>>,
    timeouts: Arc<LinearTimeout>,
    partitioner: Arc<BinomialPartitioner>,
    store: Arc<RwLock<ReplaceStore<BinomialPartitioner>>>,
    evaluator: Arc<ViewChangeEvaluator>,
}

impl ViewChangeProtocol {
    pub fn new(view_change: ViewChange, node_id: usize, validators: Validators, config: &Config, peers: HashMap<usize, Arc<Peer>>) -> Self {
        let num_validators = validators.num_groups();
        trace!("num_validators = {}", num_validators);

        trace!("validator_id = {}", node_id);
        for (&peer_id, peer) in &peers {
            trace!("peer {}: {}", peer_id, peer.peer_address());
        }

        let registry = Arc::new(ValidatorRegistry::new(validators, peers));
        let verifier = Arc::new(MultithreadedVerifier::new(
            view_change.hash_with_prefix(),
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
            node_id,
        }
    }

    // TODO: Wrap start, votes, etc. in a container type that holds the aggregation and that has
    // utility methods.
    pub fn start(signed_view_change: SignedViewChange, validators: Validators, peers: HashMap<usize, Arc<Peer>>) -> Arc<Aggregation<Self>> {
        let config = Config::default();

        // deconstruct signed view change
        let SignedViewChange {
            signature,
            message: view_change,
            signer_idx: node_id,
        } = signed_view_change;
        let node_id= node_id as usize;

        // Build our contribution from signature and our validator ID
        let contribution = IndividualSignature::new(signature, node_id);

        // Create view change protocol
        let protocol = Self::new(view_change, node_id, validators, &config, peers);

        // Create aggregation and push our contribution
        let aggregation = Aggregation::new(protocol, config);
        aggregation.push_contribution(contribution);

        aggregation
    }

    pub fn total_votes(&self) -> usize {
        let store = self.store.read();
        store.best(store.best_level())
            .map(|multisig| {
                self.registry.signature_weight(&Signature::Multi(multisig.clone()))
                    .unwrap_or_else(|| panic!("Unknown signers in signature: {:?}", multisig))
            })
            .unwrap_or(0)
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

    fn send_to(&self, to: usize, update: LevelUpdate) -> Result<(), IoError> {
        if let Some(peer) = self.registry.peer(to) {
            peer.channel.send(Message::HandelViewChange(Box::new(update.clone().with_message(self.view_change.clone()))))
                .map_err(|e| IoError::new(ErrorKind::Other, e))
        }
        else {
            //warn!("No peer for validator ID {}", to);
            Ok(())
        }
    }

    fn node_id(&self) -> usize {
        self.node_id
    }
}

impl fmt::Debug for ViewChangeProtocol {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "ViewChangeProtocol {{ node_id: {}, block_number: {}, new_view_number: {} }}", self.node_id, self.view_change.block_number, self.view_change.new_view_number)
    }
}
