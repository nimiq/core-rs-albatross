use std::sync::Arc;

use futures::future::FutureResult;
use futures::{future, Future};
use futures_cpupool::{CpuFuture, CpuPool};
use lazy_static::lazy_static;

use bls::AggregatePublicKey;
use hash::Blake2sHash;

use crate::identity::IdentityRegistry;
use crate::multisig::{IndividualSignature, MultiSignature, Signature};

lazy_static! {
    /// CPU pool that is shared between all Handel instances (that use it)
    static ref SHARED_CPU_POOL: Arc<CpuPool> = Arc::new(CpuPool::new_num_cpus());
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

/// Trait for a signature verification backend
pub trait Verifier {
    type Output: Future<Item = VerificationResult, Error = ()> + Send + Sync + 'static;

    fn verify(&self, signature: &Signature) -> Self::Output;
}

/// A dummy verifier that accepts all signatures
pub struct DummyVerifier();

impl Verifier for DummyVerifier {
    type Output = FutureResult<VerificationResult, ()>;

    fn verify(&self, _signature: &Signature) -> Self::Output {
        future::ok(VerificationResult::Ok)
    }
}

pub struct MultithreadedVerifier<I: IdentityRegistry> {
    message_hash: Blake2sHash,
    identity_registry: Arc<I>,
    cpu_pool: Arc<CpuPool>,
}

impl<I: IdentityRegistry> MultithreadedVerifier<I> {
    pub fn new(
        message_hash: Blake2sHash,
        identity_registry: Arc<I>,
        cpu_pool: Arc<CpuPool>,
    ) -> Self {
        Self {
            message_hash,
            identity_registry,
            cpu_pool,
        }
    }

    pub fn shared(message_hash: Blake2sHash, identity_registry: Arc<I>) -> Self {
        Self::new(
            message_hash,
            identity_registry,
            Arc::clone(&SHARED_CPU_POOL),
        )
    }

    fn verify_individual(
        identity_registry: Arc<I>,
        message_hash: Blake2sHash,
        individual: &IndividualSignature,
    ) -> VerificationResult {
        if let Some(public_key) = identity_registry.public_key(individual.signer) {
            if public_key.verify_hash(message_hash, &individual.signature) {
                VerificationResult::Ok
            } else {
                VerificationResult::Forged
            }
        } else {
            VerificationResult::UnknownSigner {
                signer: individual.signer,
            }
        }
    }

    fn verify_multisig(
        identity_registry: Arc<I>,
        message_hash: Blake2sHash,
        multisig: &MultiSignature,
    ) -> VerificationResult {
        let mut aggregated_public_key = AggregatePublicKey::new();
        for signer in multisig.signers.iter() {
            if let Some(public_key) = identity_registry.public_key(signer) {
                aggregated_public_key.aggregate(&public_key);
            } else {
                return VerificationResult::UnknownSigner { signer };
            }
        }

        if aggregated_public_key.verify_hash(message_hash, &multisig.signature) {
            VerificationResult::Ok
        } else {
            VerificationResult::Forged
        }
    }
}

impl<I: IdentityRegistry + Sync + Send + 'static> Verifier for MultithreadedVerifier<I> {
    type Output = CpuFuture<VerificationResult, ()>;

    fn verify(&self, signature: &Signature) -> Self::Output {
        // We clone it so that we can move it into the closure
        let signature = signature.clone();
        let message_hash = self.message_hash.clone();
        let identity_registry = Arc::clone(&self.identity_registry);

        self.cpu_pool.spawn_fn(move || {
            let res = match &signature {
                Signature::Individual(individual) => {
                    Self::verify_individual(identity_registry, message_hash, individual)
                }
                Signature::Multi(multisig) => {
                    Self::verify_multisig(identity_registry, message_hash, multisig)
                }
            };
            Ok(res)
        })
    }
}
