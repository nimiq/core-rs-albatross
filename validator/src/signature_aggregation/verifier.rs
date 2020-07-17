use std::sync::Arc;

use block_albatross::MultiSignature;
use bls::AggregatePublicKey;
use futures_cpupool::{CpuFuture, CpuPool};
use handel::identity::IdentityRegistry;
use handel::verifier::{VerificationResult, Verifier};
use hash::Blake2sHash;
use lazy_static::lazy_static;

lazy_static! {
    /// CPU pool that is shared between all Handel instances (that use it)
    static ref SHARED_CPU_POOL: Arc<CpuPool> = Arc::new(CpuPool::new_num_cpus());
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
    type Contribution = MultiSignature;

    fn verify(&self, signature: &MultiSignature) -> Self::Output {
        // We clone it so that we can move it into the closure
        let signature = signature.clone();
        let message_hash = self.message_hash.clone();
        let identity_registry = Arc::clone(&self.identity_registry);

        self.cpu_pool.spawn_fn(move || {
            let res = Self::verify_multisig(identity_registry, message_hash, &signature);

            Ok(res)
        })
    }
}
