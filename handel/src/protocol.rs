use std::sync::Arc;
use std::io::Error as IoError;

use parking_lot::RwLock;

use crate::identity::IdentityRegistry;
use crate::verifier::Verifier;
use crate::timeout::TimeoutStrategy;
use crate::store::SignatureStore;
use crate::evaluator::Evaluator;
use crate::multisig::{MultiSignature, IndividualSignature};
use crate::partitioner::Partitioner;
use crate::update::LevelUpdate;
use crate::multisig::Signature;



pub trait Protocol: Send + Sync + 'static {
    type Registry: IdentityRegistry;
    type Verifier: Verifier;
    type Timeouts: TimeoutStrategy;
    type Store: SignatureStore;
    type Evaluator: Evaluator + Send + Sync;
    type Partitioner: Partitioner;

    fn registry(&self) -> Arc<Self::Registry>;
    fn verifier(&self) -> Arc<Self::Verifier>;
    fn timeouts(&self) -> Arc<Self::Timeouts>;
    fn store(&self) -> Arc<RwLock<Self::Store>>;
    fn evaluator(&self) -> Arc<Self::Evaluator>;
    fn partitioner(&self) -> Arc<Self::Partitioner>;

    fn send_to(&self, to: usize, update: LevelUpdate) -> Result<(), IoError>;
    fn node_id(&self) -> usize;

    fn verify(&self, signature: &Signature) -> <Self::Verifier as Verifier>::Output {
        self.verifier().verify(signature)
    }
}
