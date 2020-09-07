use std::fmt;
use std::sync::Arc;

use beserial::WriteBytesExt;

// these should be moved here once they are no longer sed by the messages crate.
use block_albatross::{MultiSignature, SignedViewChange, ViewChange, ViewChangeProof};
use collections::BitSet;
use parking_lot::RwLock;

use nimiq_handel::aggregation::Aggregation;
use nimiq_handel::config::Config;
use nimiq_handel::evaluator::WeightedVote;
use nimiq_handel::partitioner::BinomialPartitioner;
use nimiq_handel::protocol::Protocol;
use nimiq_handel::store::ReplaceStore;
use nimiq_hash::{Blake2sHash, Blake2sHasher, Hasher, SerializeContent};
use nimiq_network_interface::network::Network;
use nimiq_primitives::policy;
use nimiq_primitives::slot::{SlotCollection, ValidatorSlots};

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
        // aquire lock on validartorPool and read the active validator count
        let num_validators = validators.len();

        let registry = Arc::new(ValidatorRegistry::new(validators));
        let partitioner = Arc::new(BinomialPartitioner::new(node_id, num_validators));

        let store = Arc::new(RwLock::new(ReplaceStore::<
            BinomialPartitioner,
            MultiSignature,
        >::new(partitioner.clone())));

        let evaluator = Arc::new(WeightedVote::new(
            store.clone(),
            registry.clone(),
            partitioner.clone(),
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
    type Partitioner = BinomialPartitioner;
    type Store = ReplaceStore<Self::Partitioner, Self::Contribution>;
    type Evaluator = WeightedVote<Self::Store, Self::Registry, Self::Partitioner>;

    fn verifier(&self) -> Arc<Self::Verifier> {
        self.verifier.clone()
    }

    fn registry(&self) -> Arc<Self::Registry> {
        self.registry.clone()
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
