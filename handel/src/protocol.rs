use std::sync::Arc;

use parking_lot::RwLock;

use crate::evaluator::Evaluator;
use crate::identity::IdentityRegistry;
use crate::multisig::Signature;
use crate::partitioner::Partitioner;
use crate::sender::Sender;
use crate::store::SignatureStore;
use crate::verifier::Verifier;

pub trait Protocol: Send + Sync + 'static {
    // TODO: Some of those traits can be directly move into `Protocol`. Others should not be part of
    // the protocol (i.e. `Verifier`).
    type Registry: IdentityRegistry;
    type Verifier: Verifier;
    //type Timeouts: TimeoutStrategy;
    type Store: SignatureStore;
    type Evaluator: Evaluator + Send + Sync;
    type Partitioner: Partitioner;
    type Sender: Sender;

    fn registry(&self) -> Arc<Self::Registry>;
    fn verifier(&self) -> Arc<Self::Verifier>;
    //fn timeouts(&self) -> Arc<Self::Timeouts>;
    fn store(&self) -> Arc<RwLock<Self::Store>>;
    fn evaluator(&self) -> Arc<Self::Evaluator>;
    fn partitioner(&self) -> Arc<Self::Partitioner>;
    fn sender(&self) -> Arc<Self::Sender>;

    //fn send_to(&self, to: usize, update: LevelUpdate) -> Result<(), IoError>;
    fn node_id(&self) -> usize;

    fn verify(&self, signature: &Signature) -> <Self::Verifier as Verifier>::Output {
        self.verifier().verify(signature)
    }
}
