use std::fmt;
use std::sync::Arc;

use futures::stream::StreamExt;
use parking_lot::RwLock;

use beserial::WriteBytesExt;
// these should be moved here once they are no longer used by the messages crate.
use block_albatross::{MultiSignature, SignedViewChange, ViewChange, ViewChangeProof};
use collections::BitSet;
use handel::aggregation::Aggregation;
use handel::config::Config;
use handel::contribution::AggregatableContribution;
use handel::evaluator::WeightedVote;
use handel::identity::WeightRegistry;
use handel::partitioner::BinomialPartitioner;
use handel::protocol::Protocol;
use handel::store::ReplaceStore;
use handel::update::LevelUpdateMessage;
use hash::{Blake2sHash, Blake2sHasher, Hasher, SerializeContent};
use network_interface::network::Network;
use primitives::policy;
use primitives::slot::{SlotCollection, ValidatorSlots};

use super::network_sink::NetworkSink;
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
    pub fn new(validators: ValidatorSlots, node_id: usize, threshold: usize, message_hash: Blake2sHash) -> Self {
        let partitioner = Arc::new(BinomialPartitioner::new(node_id, validators.len()));

        let store = Arc::new(RwLock::new(ReplaceStore::<BinomialPartitioner, MultiSignature>::new(Arc::clone(&partitioner))));

        let registry = Arc::new(ValidatorRegistry::new(validators));

        let evaluator = Arc::new(WeightedVote::new(
            Arc::clone(&store),
            Arc::clone(&registry),
            Arc::clone(&partitioner),
            threshold,
        ));

        ViewChangeAggregationProtocol {
            verifier: Arc::new(MultithreadedVerifier::new(message_hash, Arc::clone(&registry))),
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
    pub async fn start(view_change: SignedViewChange, validator_id: u16, active_validators: ValidatorSlots, network: Arc<N>) -> ViewChangeProof {
        let mut hasher = Blake2sHasher::new();
        hasher.write_u8(PREFIX_VIEW_CHANGE).expect("Failed to write prefix to hasher for signature.");
        view_change
            .message
            .serialize_content(&mut hasher)
            .expect("Failed to write message to hasher for signature.");
        let message_hash = hasher.finish();

        // TODO expose this somewehere else so we don't need to clone here.
        let weights = ValidatorRegistry::new(active_validators.clone());

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

        trace!(
            "Starting view change {}.{}",
            &view_change.message.block_number,
            &view_change.message.new_view_number,
        );

        let mut aggregation = Aggregation::new(
            protocol,
            view_change.message,
            Config::default(),
            own_contribution,
            Box::pin(
                network
                    .receive_from_all::<LevelUpdateMessage<MultiSignature, ViewChange>>()
                    .map(move |msg| msg.0.update),
            ),
            Box::new(NetworkSink::<
                LevelUpdateMessage<MultiSignature, ViewChange>,
                N,
            >::new(network.clone())),
        );

        while let Some(aggregate) = aggregation.next().await {
            if let Some(aggregate_weight) = weights.signature_weight(&aggregate) {
                trace!(
                    "New View Change Aggregate weight: {} / {} Signers: {:?}",
                    aggregate_weight,
                    policy::TWO_THIRD_SLOTS,
                    &aggregate.contributors(),
                );

                // Check if the combined weight of the aggregation is above the Two_THIRD_SLOTS threshold.
                if aggregate_weight > policy::TWO_THIRD_SLOTS as usize {
                    // Make sure remaining messages get send.
                    tokio::spawn(async move { aggregation.shutdown().await });

                    // Create ViewChnageProof out of the aggregate
                    let view_change_proof =
                        ViewChangeProof::new(aggregate.signature, aggregate.signers);
                    trace!("View Change complete: {:?}", &view_change_proof);

                    // return the ViewChangeProof
                    return view_change_proof;
                }
            }
        }
        panic!("ViewChangeAggregation Stream returned Ready(None) without a completed ViewChange");
    }
}

impl fmt::Debug for ViewChangeAggregationProtocol {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "ViewChangeAggregation {{ node_id: {} }}", self.node_id(),)
    }
}
