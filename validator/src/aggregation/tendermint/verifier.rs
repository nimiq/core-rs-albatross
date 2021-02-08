use std::sync::Arc;

use async_trait::async_trait;
use tokio::task;

use nimiq_block_albatross::{TendermintIdentifier, TendermintVote};
use nimiq_bls::AggregatePublicKey;
use nimiq_handel::identity::IdentityRegistry;
use nimiq_handel::verifier::{VerificationResult, Verifier};
use nimiq_hash::Hash;

use super::contribution::TendermintContribution;

#[derive(Debug)]
pub(crate) struct TendermintVerifier<I: IdentityRegistry> {
    identity_registry: Arc<I>,
    id: TendermintIdentifier,
    validator_merkle_root: Vec<u8>,
}

impl<I: IdentityRegistry> TendermintVerifier<I> {
    pub(crate) fn new(identity_registry: Arc<I>, id: TendermintIdentifier, validator_merkle_root: Vec<u8>) -> Self {
        Self {
            identity_registry,
            id,
            validator_merkle_root,
        }
    }
}

#[async_trait]
impl<I: IdentityRegistry + Sync + Send + 'static> Verifier for TendermintVerifier<I> {
    // type Output = CpuFuture<VerificationResult, ()>;
    type Contribution = TendermintContribution;

    async fn verify(&self, contribution: &Self::Contribution) -> VerificationResult {
        // Store the JoinHandles so they can be awaited later.
        let mut results: Vec<task::JoinHandle<VerificationResult>> = vec![];

        // Every different proposals contributions must be verified.
        // Note: Once spawned the tasks cannot be aborted. Thus all contributions will be verified even though it is not strictly necessary.
        // I.e once f contributions are against the proposal this node signed, it is already known that that proposal is not going to pass.
        // Likewise once a proposal has 2f+1 valid contributions it passed and no other contributions are needed.
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
            let contribution = multi_sig.clone();
            let vote = TendermintVote {
                id: self.id.clone(),
                proposal_hash: hash.clone(),
                validator_merkle_root: self.validator_merkle_root.clone(),
            };

            results.push(task::spawn_blocking(move || {
                if aggregated_public_key.verify_hash(vote.hash(), &contribution.signature) {
                    VerificationResult::Ok
                } else {
                    VerificationResult::Forged
                }
            }));
        }

        // Take all the join handles and await them.
        // If there is a single join handle which returned a VerifactionResult != VerificationResult::Ok
        // this Verificatioon failed as well with the given VerificationResult.
        // let vec_iter = stream::iter(results);
        while let Some(handle) = results.pop() {
            let result = handle.await.expect("Spawned Verification Task has paniced");
            if !result.is_ok() {
                return result;
            }
        }

        // All results were awaited and Ok. Verification is Ok.
        VerificationResult::Ok
    }
}
