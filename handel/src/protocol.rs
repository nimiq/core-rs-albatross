use async_trait::async_trait;
use std::sync::Arc;

use parking_lot::RwLock;

use crate::contribution::AggregatableContribution;
use crate::evaluator::Evaluator;
use crate::identity::IdentityRegistry;
use crate::partitioner::Partitioner;
use crate::store::ContributionStore;
use crate::verifier::{VerificationResult, Verifier};

#[async_trait]
pub trait Protocol: Send + Sync + 'static {
    /// The type foor individual as well as aggregated contributions.
    type Contribution: AggregatableContribution;
    // TODO: Some of those traits can be directly move into `Protocol`. Others should not be part of
    // the protocol (i.e. `Verifier`).
    type Registry: IdentityRegistry;
    type Verifier: Verifier<Contribution = Self::Contribution>;
    type Store: ContributionStore<Contribution = Self::Contribution>;
    type Evaluator: Evaluator<Self::Contribution>;
    type Partitioner: Partitioner;

    fn registry(&self) -> Arc<Self::Registry>;
    fn verifier(&self) -> Arc<Self::Verifier>;
    fn store(&self) -> Arc<RwLock<Self::Store>>;
    fn evaluator(&self) -> Arc<Self::Evaluator>;
    fn partitioner(&self) -> Arc<Self::Partitioner>;

    //fn send_to(&self, to: usize, update: LevelUpdate) -> Result<(), IoError>;
    fn node_id(&self) -> usize;

    // TODO: not strictly necessary as it does the same as protocol.verifier().verify(contribution).
    async fn verify(&self, contribution: &Self::Contribution) -> VerificationResult {
        self.verifier().verify(contribution).await
    }
}
