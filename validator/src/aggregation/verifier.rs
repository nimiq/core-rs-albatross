use std::sync::Arc;

use async_trait::async_trait;

use bls::AggregatePublicKey;
use handel::identity::IdentityRegistry;
use handel::verifier::{VerificationResult, Verifier};
use hash::Blake2sHash;
use super::view_change::SignedViewChangeMessage;

pub struct MultithreadedVerifier<I: IdentityRegistry> {
    message_hash: Blake2sHash,
    identity_registry: Arc<I>,
}

impl<I: IdentityRegistry> MultithreadedVerifier<I> {
    pub fn new(message_hash: Blake2sHash, identity_registry: Arc<I>) -> Self {
        Self {
            message_hash,
            identity_registry,
        }
    }
}

#[async_trait]
impl<I: IdentityRegistry + Sync + Send + 'static> Verifier for MultithreadedVerifier<I> {
    // type Output = CpuFuture<VerificationResult, ()>;
    type Contribution = SignedViewChangeMessage;

    async fn verify(&self, contribution: &Self::Contribution) -> VerificationResult {
        let mut aggregated_public_key = AggregatePublicKey::new();
        for signer in contribution.view_change.signers.iter() {
            if let Some(public_key) = self.identity_registry.public_key(signer) {
                aggregated_public_key.aggregate(&public_key);
            } else {
                warn!("Signer public key not found");
                return VerificationResult::UnknownSigner { signer };
            }
        }

        if aggregated_public_key.verify_hash(self.message_hash.clone(), &contribution.view_change.signature) {
            VerificationResult::Ok
        } else {
            VerificationResult::Forged
        }
    }
}
