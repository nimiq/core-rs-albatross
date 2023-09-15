use std::sync::Arc;

use async_trait::async_trait;
use nimiq_bls::AggregatePublicKey;
use nimiq_handel::{
    identity::IdentityRegistry,
    verifier::{VerificationResult, Verifier},
};
use nimiq_hash::Hash;
use nimiq_primitives::{TendermintIdentifier, TendermintVote};
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use tokio::task;

use super::contribution::TendermintContribution;

#[derive(Debug)]
pub(crate) struct TendermintVerifier<I: IdentityRegistry> {
    identity_registry: Arc<I>,
    id: TendermintIdentifier,
}

impl<I: IdentityRegistry> TendermintVerifier<I> {
    pub(crate) fn new(identity_registry: Arc<I>, id: TendermintIdentifier) -> Self {
        Self {
            identity_registry,
            id,
        }
    }
}

#[async_trait]
impl<I: IdentityRegistry + Sync + Send + 'static> Verifier for TendermintVerifier<I> {
    // type Output = CpuFuture<VerificationResult, ()>;
    type Contribution = TendermintContribution;

    async fn verify(&self, contribution: &Self::Contribution) -> VerificationResult {
        // Every different proposals contributions must be verified.
        // Note: Once spawned the tasks cannot be aborted. Thus all contributions will be verified even though it is not strictly necessary.
        // I.e once f contributions are against the proposal this node signed, it is already known that that proposal is not going to pass.
        // Likewise once a proposal has 2f+1 valid contributions it passed and no other contributions are needed.
        let mut params = Vec::new();
        for (hash, multi_sig) in &contribution.contributions {
            // Create  the aggregated public key for this specific proposal hash's contributions.
            let mut aggregated_public_key = AggregatePublicKey::new();
            for signer in multi_sig.signers.iter() {
                if let Some(public_key) = self.identity_registry.public_key(signer) {
                    aggregated_public_key.aggregate(&public_key);
                } else {
                    return VerificationResult::UnknownSigner { signer };
                }
            }

            // create a thread that is allowed to block for this specific proposals hash contributions.
            let vote = TendermintVote {
                id: self.id.clone(),
                proposal_hash: hash.clone(),
            };

            params.push((aggregated_public_key, vote, multi_sig.clone()));
        }
        let result = task::spawn_blocking(move || {
            params
                .into_par_iter()
                .map(|(aggregated_public_key, vote, contribution)| {
                    if aggregated_public_key.verify_hash(vote.hash(), &contribution.signature) {
                        Ok(())
                    } else {
                        Err(())
                    }
                })
                // If there is a single verification that failed, fail the whole verification as well.
                .try_reduce(|| (), |(), ()| Ok(()))
        })
        .await
        .expect("spawned verification task has panicked");

        match result {
            // All results were Ok. Verification is Ok.
            Ok(()) => VerificationResult::Ok,
            Err(()) => VerificationResult::Forged,
        }
    }
}
