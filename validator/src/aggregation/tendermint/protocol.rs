use std::sync::Arc;

use nimiq_handel::{
    evaluator::WeightedVote, partitioner::BinomialPartitioner, protocol::Protocol,
    store::ReplaceStore,
};
use nimiq_primitives::TendermintIdentifier;
use parking_lot::RwLock;

use super::{
    super::registry::ValidatorRegistry, contribution::TendermintContribution,
    verifier::TendermintVerifier,
};

#[derive(std::fmt::Debug)]
pub(crate) struct TendermintAggregationProtocol {
    verifier: Arc<<Self as Protocol<TendermintIdentifier>>::Verifier>,
    partitioner: Arc<<Self as Protocol<TendermintIdentifier>>::Partitioner>,
    evaluator: Arc<<Self as Protocol<TendermintIdentifier>>::Evaluator>,
    store: Arc<RwLock<<Self as Protocol<TendermintIdentifier>>::Store>>,
    registry: Arc<<Self as Protocol<TendermintIdentifier>>::Registry>,

    node_id: usize,
    id: TendermintIdentifier,
}

impl TendermintAggregationProtocol {
    pub(crate) fn new(
        validators: Arc<ValidatorRegistry>,
        node_id: usize,
        threshold: usize,
        id: TendermintIdentifier,
    ) -> Self {
        let partitioner = Arc::new(BinomialPartitioner::new(node_id, validators.len()));

        let store = Arc::new(RwLock::new(
            ReplaceStore::<TendermintIdentifier, Self>::new(Arc::clone(&partitioner)),
        ));

        let evaluator = Arc::new(WeightedVote::new(
            Arc::clone(&store),
            validators.clone(),
            Arc::clone(&partitioner),
            threshold,
        ));

        let verifier = Arc::new(TendermintVerifier::new(validators.clone(), id.clone()));

        Self {
            verifier,
            partitioner,
            evaluator,
            store,
            registry: validators,
            node_id,
            id,
        }
    }
}

impl Protocol<TendermintIdentifier> for TendermintAggregationProtocol {
    type Contribution = TendermintContribution;
    type Registry = ValidatorRegistry;
    type Verifier = TendermintVerifier<Self::Registry>;
    type Store = ReplaceStore<TendermintIdentifier, Self>;
    type Evaluator = WeightedVote<TendermintIdentifier, Self>;
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

    fn identify(&self) -> TendermintIdentifier {
        self.id.clone()
    }

    fn node_id(&self) -> usize {
        self.node_id
    }
}
