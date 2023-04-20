use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::RwLock;

use crate::contribution::AggregatableContribution;
use crate::evaluator::Evaluator;
use crate::identity::{IdentityRegistry, WeightRegistry};
use crate::partitioner::Partitioner;
use crate::store::ContributionStore;
use crate::verifier::{VerificationResult, Verifier};

#[async_trait]
pub trait Protocol<TId: Clone + std::fmt::Debug + 'static>: Send + Sync + Sized + 'static {
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

    // TODO: not strictly necessary as it does the same as protocol.verifier().verify(contribution).
    async fn verify(&self, contribution: &Self::Contribution) -> VerificationResult {
        self.verifier().verify(contribution).await
    }
}
