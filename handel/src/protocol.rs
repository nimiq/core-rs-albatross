use std::sync::Arc;

use parking_lot::RwLock;

use crate::{
    contribution::AggregatableContribution,
    evaluator::Evaluator,
    identity::{IdentityRegistry, WeightRegistry},
    partitioner::Partitioner,
    store::ContributionStore,
    verifier::Verifier,
    Identifier,
};

pub trait Protocol<TId>
where
    TId: Identifier,
    Self: Send + Sync + Sized + Unpin + 'static,
{
    /// The type for individual as well as aggregated contributions.
    type Contribution: AggregatableContribution;
    // TODO: Some of those traits can be directly move into `Protocol`. Others should not be part of
    // the protocol (i.e. `Verifier`).
    type Registry: IdentityRegistry + WeightRegistry;
    type Verifier: Verifier<Contribution = Self::Contribution>;
    type Store: ContributionStore<TId, Self>;
    type Evaluator: Evaluator<TId, Self>;
    type Partitioner: Partitioner;

    fn registry(&self) -> Arc<Self::Registry>;
    fn verifier(&self) -> Arc<Self::Verifier>;
    fn store(&self) -> Arc<RwLock<Self::Store>>;
    fn evaluator(&self) -> Arc<Self::Evaluator>;
    fn partitioner(&self) -> Arc<Self::Partitioner>;

    fn identify(&self) -> TId;
    fn node_id(&self) -> usize;
}
