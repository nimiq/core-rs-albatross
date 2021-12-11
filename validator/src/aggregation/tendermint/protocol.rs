use parking_lot::RwLock;
use std::sync::Arc;

use nimiq_block::TendermintIdentifier;
use nimiq_handel::evaluator::WeightedVote;
use nimiq_handel::partitioner::BinomialPartitioner;
use nimiq_handel::protocol::Protocol;
use nimiq_handel::store::ReplaceStore;

use super::super::registry::ValidatorRegistry;

use super::contribution::TendermintContribution;
use super::verifier::TendermintVerifier;

#[derive(std::fmt::Debug)]
pub(crate) struct TendermintAggregationProtocol {
    verifier: Arc<<Self as Protocol>::Verifier>,
    partitioner: Arc<<Self as Protocol>::Partitioner>,
    evaluator: Arc<<Self as Protocol>::Evaluator>,
    store: Arc<RwLock<<Self as Protocol>::Store>>,
    registry: Arc<<Self as Protocol>::Registry>,

    node_id: usize,
}

impl TendermintAggregationProtocol {
    pub(super) fn new(
        validators: Arc<ValidatorRegistry>,
        node_id: usize,
        threshold: usize,
        id: TendermintIdentifier,
    ) -> Self {
        let partitioner = Arc::new(BinomialPartitioner::new(node_id, validators.len()));

        let store = Arc::new(RwLock::new(ReplaceStore::<
            BinomialPartitioner,
            <Self as Protocol>::Contribution,
        >::new(Arc::clone(&partitioner))));

        let evaluator = Arc::new(WeightedVote::new(
            Arc::clone(&store),
            validators.clone(),
            Arc::clone(&partitioner),
            threshold,
        ));

        let verifier = Arc::new(TendermintVerifier::new(validators.clone(), id));

        Self {
            verifier,
            partitioner,
            evaluator,
            store,
            registry: validators,
            node_id,
        }
    }
}

impl Protocol for TendermintAggregationProtocol {
    type Contribution = TendermintContribution;
    type Registry = ValidatorRegistry;
    type Verifier = TendermintVerifier<Self::Registry>;
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
