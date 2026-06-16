use std::sync::Arc;

use async_trait::async_trait;
use nimiq_bls::{AggregatePublicKey, G1Projective, Signature};
use nimiq_handel::{
    identity::IdentityRegistry,
    verifier::{VerificationResult, Verifier},
};
use nimiq_hash::Blake2sHash;

use super::skip_block::SignedSkipBlockMessage;

pub struct MultithreadedVerifier<I: IdentityRegistry> {
    /// The `G1` point the message hashes to, precomputed once instead of on every contribution.
    hash_curve: G1Projective,
    identity_registry: Arc<I>,
}

impl<I: IdentityRegistry> MultithreadedVerifier<I> {
    pub fn new(message_hash: Blake2sHash, identity_registry: Arc<I>) -> Self {
        Self {
            hash_curve: Signature::hash_to_g1(message_hash),
            identity_registry,
        }
    }
}

#[async_trait]
impl<I: IdentityRegistry + Sync + Send + 'static> Verifier for MultithreadedVerifier<I> {
    type Contribution = SignedSkipBlockMessage;

    async fn verify(&self, contribution: &Self::Contribution) -> VerificationResult {
        let mut aggregated_public_key = AggregatePublicKey::new();
        for signer in contribution.proof.signers.iter() {
            if let Some(public_key) = self.identity_registry.public_key(signer) {
                aggregated_public_key.aggregate(&public_key);
            } else {
                warn!("Signer public key not found");
                return VerificationResult::UnknownSigner { signer };
            }
        }

        if aggregated_public_key.verify_g1(self.hash_curve, &contribution.proof.signature) {
            VerificationResult::Ok
        } else {
            VerificationResult::Forged
        }
    }
}
