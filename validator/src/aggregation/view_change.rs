use std::fmt;
use std::sync::Arc;

use parking_lot::RwLock;

use beserial::WriteBytesExt;
// these should be moved here once they are no longer used by the messages crate.
use block_albatross::{MultiSignature, SignedViewChange, ViewChange, ViewChangeProof};
use collections::BitSet;
use handel::aggregation::Aggregation;
use handel::config::Config;
use handel::evaluator::WeightedVote;
use handel::partitioner::BinomialPartitioner;
use handel::protocol::Protocol;
use handel::store::ReplaceStore;
use hash::{Blake2sHash, Blake2sHasher, Hasher, SerializeContent};
use network_interface::network::Network;
use primitives::policy;
use primitives::slot::{SlotCollection, ValidatorSlots};

use super::registry::ValidatorRegistry;
use super::verifier::MultithreadedVerifier;

/// prefix to sign view change messages
const PREFIX_VIEW_CHANGE: u8 = 0x01;

struct ViewChangeAggregationProtocol {
    verifier: Arc<<Self as Protocol>::Verifier>,
    partitioner: Arc<<Self as Protocol>::Partitioner>,
    evaluator: Arc<<Self as Protocol>::Evaluator>,
    store: Arc<RwLock<<Self as Protocol>::Store>>,
    registry: Arc<<Self as Protocol>::Registry>,

    node_id: usize,
}

impl ViewChangeAggregationProtocol {
    pub fn new(
        validators: ValidatorSlots,
        node_id: usize,
        threshold: usize,
        message_hash: Blake2sHash,
    ) -> Self {
        let partitioner = Arc::new(BinomialPartitioner::new(node_id, validators.len()));

        let store = Arc::new(RwLock::new(ReplaceStore::<
            BinomialPartitioner,
            MultiSignature,
        >::new(Arc::clone(&partitioner))));

        let registry = Arc::new(ValidatorRegistry::new(validators));

        let evaluator = Arc::new(WeightedVote::new(
            Arc::clone(&store),
            Arc::clone(&registry),
            Arc::clone(&partitioner),
            threshold,
        ));

        ViewChangeAggregationProtocol {
            verifier: Arc::new(MultithreadedVerifier::new(
                message_hash,
                Arc::clone(&registry),
            )),
            partitioner,
            evaluator,
            store,
            registry,
            node_id,
        }
    }
}

impl Protocol for ViewChangeAggregationProtocol {
    type Contribution = MultiSignature;
    type Registry = ValidatorRegistry;
    type Verifier = MultithreadedVerifier<Self::Registry>;
    type Store = ReplaceStore<Self::Partitioner, Self::Contribution>;
    type Evaluator = WeightedVote<Self::Store, Self::Registry, Self::Partitioner>;
    type Partitioner = BinomialPartitioner;

    fn registry(&self) -> Arc<Self::Registry> {
        self.registry.clone()
    }

    fn verifier(&self) -> Arc<Self::Verifier> {
        self.verifier.clone()
    }

    fn store(&self) -> Arc<RwLock<Self::Store>> {
        self.store.clone()
    }

    fn evaluator(&self) -> Arc<Self::Evaluator> {
        self.evaluator.clone()
    }

    fn partitioner(&self) -> Arc<Self::Partitioner> {
        self.partitioner.clone()
    }

    fn node_id(&self) -> usize {
        self.node_id
    }
}

pub struct ViewChangeAggregation<N: Network> {
    _phantom: std::marker::PhantomData<N>,
}

impl<N: Network> ViewChangeAggregation<N> {
    pub async fn start(
        view_change: SignedViewChange,
        validator_id: u16,
        active_validators: ValidatorSlots,
        network: Arc<N>,
    ) -> ViewChangeProof {
        let mut hasher = Blake2sHasher::new();
        hasher
            .write_u8(PREFIX_VIEW_CHANGE)
            .expect("Failed to write prefix to hasher for signature.");
        view_change
            .message
            .serialize_content(&mut hasher)
            .expect("Failed to write message to hasher for signature.");
        let message_hash = hasher.finish();

        let protocol = ViewChangeAggregationProtocol::new(
            active_validators,
            validator_id as usize,
            policy::TWO_THIRD_SLOTS as usize,
            message_hash,
        );

        let mut signature = bls::AggregateSignature::new();
        signature.aggregate(&view_change.signature);

        let mut signers = BitSet::new();
        signers.insert(validator_id as usize);

        let own_contribution = MultiSignature::new(signature, signers);

        let multi_signature = Aggregation::<ViewChangeAggregationProtocol, N, ViewChange>::start(
            view_change.message,
            own_contribution,
            protocol,
            Config::default(),
            network,
        )
        .await;

        // Transform into finished ViewChange
        ViewChangeProof::new(multi_signature.signature, multi_signature.signers)
    }
}

impl fmt::Debug for ViewChangeAggregationProtocol {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "ViewChangeAggregation {{ node_id: {} }}", self.node_id(),)
    }
}
