use std::sync::Arc;

use nimiq_handel::{
    evaluator::WeightedVote, identity::WeightRegistry, partitioner::BinomialPartitioner,
    protocol::Protocol, store::ReplaceStore,
};
use nimiq_primitives::{policy, TendermintIdentifier};
use parking_lot::RwLock;

use super::{
    super::registry::ValidatorRegistry, contribution::TendermintContribution,
    verifier::TendermintVerifier,
};

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
        id: TendermintIdentifier,
    ) -> Self {
        let partitioner = Arc::new(BinomialPartitioner::new(node_id, validators.len()));

        let store = Arc::new(RwLock::new(
            ReplaceStore::<TendermintIdentifier, Self>::new(Arc::clone(&partitioner)),
        ));

        // This will be moved into the closure.
        let cloned_validators = Arc::clone(&validators);

        let evaluator = Arc::new(WeightedVote::new(
            Arc::clone(&store),
            Arc::clone(&validators),
            Arc::clone(&partitioner),
            move |message: &TendermintContribution| {
                // Conditions for final contributions:
                // * Any proposal has 2f+1
                // (* Nil has f+1) would be implicit in the next condition
                // * The biggest proposal (NOT Nil) combined with the non cast votes is less than 2f+1

                let mut uncast_votes = policy::Policy::SLOTS as usize;
                let mut biggest_weight = 0;

                // Check every proposal present, including None
                for (hash, signature) in message.contributions.iter() {
                    let weight = cloned_validators
                        .signers_weight(&signature.signers)
                        .unwrap_or(0);
                    // Any option which has at least 2f+1 votes make this contribution final.
                    if weight >= policy::Policy::TWO_F_PLUS_ONE as usize {
                        return true;
                    }
                    // If the proposal does not have 2f+1 check if it has the new biggest_weight
                    if hash.is_some() && weight > biggest_weight {
                        biggest_weight = weight;
                    }
                    // The cast votes need to be removed from the uncast votes
                    uncast_votes = uncast_votes.saturating_sub(weight);
                }

                let cast_votes = (policy::Policy::SLOTS as usize).saturating_sub(uncast_votes);
                if cast_votes < policy::Policy::TWO_F_PLUS_ONE as usize {
                    // This should not be here. The criteria that the best vote can be elevated above the threshold
                    // using the uncast votes is enough. In our case that means that seeing f+1 different proposals
                    // with 1 vote each is a final aggregate. That is currently not handled that way in tendermint
                    // as it only looks at contributions above 2f contributors. That behavior is thus maintained
                    // here, but can be removed in the future if tendermint is changed as well.
                    return false;
                }

                // If the uncast votes are not able to put the biggest vote over the threshold
                // they cannot push any proposal over the threshold and thus the aggregate is final.
                if uncast_votes + biggest_weight < policy::Policy::TWO_F_PLUS_ONE as usize {
                    return true;
                }

                false
            },
        ));

        let verifier = Arc::new(TendermintVerifier::new(Arc::clone(&validators), id.clone()));

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
