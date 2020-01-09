use std::sync::Arc;

use futures::FutureExt;
use futures::future::BoxFuture;
use futures::channel::oneshot::channel;
use futures::executor::ThreadPool;
use lazy_static::lazy_static;

use hash::Blake2bHash;
use bls::bls12_381::AggregatePublicKey;

use crate::multisig::{Signature, IndividualSignature, MultiSignature};
use crate::identity::IdentityRegistry;



lazy_static! {
    /// CPU pool that is shared between all Handel instances (that use it)
    static ref SHARED_CPU_POOL: Arc<ThreadPool> = {
        let pool = ThreadPool::new()
            .expect("Failed to create Handel thread pool");
        Arc::new(pool)
    };
}


#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VerificationResult {
    Ok,
    UnknownSigner { signer: usize },
    Forged,
}

impl VerificationResult {
    pub fn is_ok(&self) -> bool {
        *self == VerificationResult::Ok
    }
}

pub type VerificationFuture = BoxFuture<'static, VerificationResult>;

/// Trait for a signature verification backend
pub trait Verifier {
    fn verify(&self, signature: &Signature) -> VerificationFuture;
}


/// A dummy verifier that accepts all signatures
pub struct DummyVerifier();

impl Verifier for DummyVerifier {
    fn verify(&self, _signature: &Signature) -> VerificationFuture {
        async { VerificationResult::Ok }.boxed()
    }
}



pub struct MultithreadedVerifier<I: IdentityRegistry> {
    message_hash: Blake2bHash,
    identity_registry: Arc<I>,
    cpu_pool: Arc<ThreadPool>,
}

impl<I: IdentityRegistry> MultithreadedVerifier<I> {
    pub fn new(message_hash: Blake2bHash, identity_registry: Arc<I>, cpu_pool: Arc<ThreadPool>) -> Self {
        Self {
            message_hash,
            identity_registry,
            cpu_pool,
        }
    }

    pub fn shared(message_hash: Blake2bHash, identity_registry: Arc<I>) -> Self {
        Self::new(message_hash, identity_registry, Arc::clone(&SHARED_CPU_POOL))
    }

    fn verify_individual(identity_registry: Arc<I>, message_hash: Blake2bHash, individual: &IndividualSignature) -> VerificationResult {
        if let Some(public_key) = identity_registry.public_key(individual.signer) {
            if public_key.verify_hash(message_hash, &individual.signature) {
                VerificationResult::Ok
            }
            else {
                VerificationResult::Forged
            }
        }
        else {
            VerificationResult::UnknownSigner { signer: individual.signer }
        }
    }

    fn verify_multisig(identity_registry: Arc<I>, message_hash: Blake2bHash, multisig: &MultiSignature) -> VerificationResult {
        let mut aggregated_public_key = AggregatePublicKey::new();
        for signer in multisig.signers.iter() {
            if let Some(public_key) = identity_registry.public_key(signer) {
                aggregated_public_key.aggregate(&public_key);
            }
            else {
                return VerificationResult::UnknownSigner { signer }
            }
        }

        if aggregated_public_key.verify_hash(message_hash, &multisig.signature) {
            VerificationResult::Ok
        }
        else {
            VerificationResult::Forged
        }
    }
}

impl<I: IdentityRegistry + Sync + Send + 'static> Verifier for MultithreadedVerifier<I> {
    fn verify(&self, signature: &Signature) -> VerificationFuture {
        // We clone it so that we can move it into the closure
        let signature = signature.clone();
        let message_hash = self.message_hash.clone();
        let identity_registry = Arc::clone(&self.identity_registry);

        let (tx, rx) = channel();
        self.cpu_pool.spawn_ok(async move {
            let res = match &signature {
                Signature::Individual(individual) => {
                    Self::verify_individual(identity_registry, message_hash, individual)
                },
                Signature::Multi(multisig) => {
                    Self::verify_multisig(identity_registry, message_hash, multisig)
                }
            };
            tx.send(res)
                .expect("Failed to send verification result");
        });
        async {
            rx.await
                .expect("Failed to receive verification result")
        }.boxed()
    }
}
