use std::sync::Arc;

use futures::{future, Future};
use futures::future::FutureResult;
use futures_cpupool::{CpuPool, CpuFuture};

use hash::Blake2bHash;
use bls::bls12_381::AggregatePublicKey;

use crate::multisig::{Signature, IndividualSignature, MultiSignature};
use crate::identity::IdentityRegistry;



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
    type Output: Future<Item=VerificationResult, Error=()> + Send + Sync + 'static;

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
    message_hash: Blake2bHash,
    identity_registry: Arc<I>,
    workers: CpuPool,
}

impl<I: IdentityRegistry> MultithreadedVerifier<I> {
    pub fn new(message_hash: Blake2bHash, identity_registry: Arc<I>, num_workers: Option<usize>) -> Self {
        let workers = if let Some(n) = num_workers {
            CpuPool::new(n)
        } else {
            CpuPool::new_num_cpus()
        };

        Self {
            message_hash,
            identity_registry,
            workers,
        }
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
    type Output = CpuFuture<VerificationResult, ()>;

    fn verify(&self, signature: &Signature) -> Self::Output {
        // We clone it so that we can move it into the closure
        let signature = signature.clone();
        let message_hash = self.message_hash.clone();
        let identity_registry = Arc::clone(&self.identity_registry);

        self.workers.spawn_fn(move || {
            Ok(match signature {
                Signature::Individual(ref individual) => {
                    Self::verify_individual(identity_registry, message_hash, individual)
                },
                Signature::Multi(ref multisig) => {
                    Self::verify_multisig(identity_registry, message_hash, multisig)
                }
            })
        })
    }
}
