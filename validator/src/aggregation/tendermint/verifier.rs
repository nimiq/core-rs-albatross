use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use nimiq_bls::{AggregatePublicKey, G1Projective, Signature};
use nimiq_collections::bitset::BitSet;
use nimiq_handel::{
    identity::IdentityRegistry,
    verifier::{VerificationResult, Verifier},
};
use nimiq_hash::{Blake2sHash, Hash};
use nimiq_primitives::{TendermintIdentifier, TendermintVote};
use parking_lot::Mutex;
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use tokio::task;

use super::contribution::TendermintContribution;

#[derive(Debug)]
pub(crate) struct TendermintVerifier<I: IdentityRegistry> {
    identity_registry: Arc<I>,
    id: TendermintIdentifier,
    /// Caches, per proposal hash, the `G1` point its vote hashes to. The `id` is fixed for the
    /// verifier, so the same proposal recurs across the many contributions of a round and the
    /// hash-to-curve only needs to be computed once per distinct proposal.
    hash_curves: Mutex<HashMap<Option<Blake2sHash>, G1Projective>>,
}

impl<I: IdentityRegistry> TendermintVerifier<I> {
    pub(crate) fn new(identity_registry: Arc<I>, id: TendermintIdentifier) -> Self {
        Self {
            identity_registry,
            id,
            hash_curves: Mutex::new(HashMap::new()),
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
        // Tracks the signers seen across all proposal buckets so far. A given slot may only
        // contribute to a single bucket: the same slot in two buckets means the validator signed
        // conflicting votes (equivocation) or the aggregate is malformed. Either way the
        // contributor sets are no longer disjoint, which would break the union/sum invariant the
        // Tendermint state machine relies on (`aggregate()`) and underflow its contributor
        // arithmetic. Reject such contributions outright
        let mut seen_signers = BitSet::new();
        for (hash, multi_sig) in &contribution.contributions {
            // Reject the contribution if this bucket overlaps any previously seen bucket
            if seen_signers.intersection_size(&multi_sig.signers) > 0 {
                return VerificationResult::Forged;
            }
            seen_signers |= multi_sig.signers.clone();

            // Create  the aggregated public key for this specific proposal hash's contributions.
            let mut aggregated_public_key = AggregatePublicKey::new();
            for signer in multi_sig.signers.iter() {
                if let Some(public_key) = self.identity_registry.public_key(signer) {
                    aggregated_public_key.aggregate(&public_key);
                } else {
                    return VerificationResult::UnknownSigner { signer };
                }
            }

            // Hash the vote to its `G1` point, reusing the cached point for this proposal hash.
            let hash_curve = *self
                .hash_curves
                .lock()
                .entry(hash.clone())
                .or_insert_with(|| {
                    let vote = TendermintVote {
                        id: self.id.clone(),
                        proposal_hash: hash.clone(),
                    };
                    Signature::hash_to_g1(vote.hash())
                });

            params.push((aggregated_public_key, hash_curve, multi_sig.clone()));
        }
        let result = task::spawn_blocking(move || {
            params
                .into_par_iter()
                .map(|(aggregated_public_key, hash_curve, contribution)| {
                    if aggregated_public_key.verify_g1(hash_curve, &contribution.signature) {
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

#[cfg(test)]
mod tests {
    use std::{future::Future, sync::Arc};

    use nimiq_bls::KeyPair as BlsKeyPair;
    use nimiq_handel::verifier::{VerificationResult, Verifier};
    use nimiq_hash::Blake2sHash;
    use nimiq_keys::{Address, Ed25519PublicKey};
    use nimiq_primitives::{
        networks::NetworkId, slots_allocation::ValidatorsBuilder, TendermintIdentifier,
        TendermintStep, TendermintVote,
    };
    use nimiq_utils::key_rng::SecureGenerate;
    use rand::{rngs::StdRng, SeedableRng};

    use super::TendermintVerifier;
    use crate::aggregation::{
        registry::ValidatorRegistry, tendermint::contribution::TendermintContribution,
    };

    /// Builds a registry with two single-slot validators (slot 0 and slot 1), returning it
    /// alongside their voting key pairs and a shared aggregation identifier.
    fn setup() -> (
        Arc<ValidatorRegistry>,
        BlsKeyPair,
        BlsKeyPair,
        TendermintIdentifier,
    ) {
        let mut rng = StdRng::from_rng(&mut rand::rng());
        let kp0 = BlsKeyPair::generate(&mut rng);
        let kp1 = BlsKeyPair::generate(&mut rng);

        // `ValidatorsBuilder` assigns slots in address order and `START_ADDRESS < END_ADDRESS`,
        // so `kp0` gets slot 0 and `kp1` gets slot 1.
        let mut builder = ValidatorsBuilder::new();
        builder.push(
            Address::START_ADDRESS,
            kp0.public_key,
            Ed25519PublicKey::default(),
        );
        builder.push(
            Address::END_ADDRESS,
            kp1.public_key,
            Ed25519PublicKey::default(),
        );
        let registry = Arc::new(ValidatorRegistry::new(builder.build()));

        let id = TendermintIdentifier {
            network: NetworkId::UnitAlbatross,
            block_number: 1,
            round_number: 0,
            step: TendermintStep::PreVote,
        };

        (registry, kp0, kp1, id)
    }

    fn block_on<F: Future>(future: F) -> F::Output {
        // The verifier uses `tokio::task::spawn_blocking`, so it needs a Tokio context
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(future)
    }

    /// A contribution where the same slot appears in two different proposal buckets (the validator
    /// equivocated, or the aggregate was crafted) must be rejected, even though each bucket
    /// signature is individually valid. This guards the contributor-count arithmetic in
    /// `nimiq_tendermint::Tendermint::aggregate` against underflow on non-disjoint contributor sets
    #[test]
    fn rejects_contribution_with_overlapping_buckets() {
        let (registry, kp0, _kp1, id) = setup();
        let verifier = TendermintVerifier::new(registry, id.clone());

        let proposal = Blake2sHash::from([1u8; 32]);
        let vote_proposal = TendermintVote {
            proposal_hash: Some(proposal),
            id: id.clone(),
        };
        let vote_none = TendermintVote {
            proposal_hash: None,
            id,
        };

        // Slot 0 signs BOTH the proposal and None: two individually valid bucket signatures.
        let proposal_bucket =
            TendermintContribution::from_vote(vote_proposal, &kp0.secret_key, 0..1);
        let none_bucket = TendermintContribution::from_vote(vote_none, &kp0.secret_key, 0..1);

        // Merge them so slot 0 ends up in both buckets of a single contribution.
        let mut overlapping = proposal_bucket;
        overlapping.contributions.extend(none_bucket.contributions);

        assert_eq!(
            block_on(verifier.verify(&overlapping)),
            VerificationResult::Forged,
        );
    }

    /// A contribution with disjoint buckets and valid signatures must verify. This confirms the
    /// rejection above is caused by the bucket overlap, not by invalid signatures.
    #[test]
    fn accepts_contribution_with_disjoint_buckets() {
        let (registry, kp0, kp1, id) = setup();
        let verifier = TendermintVerifier::new(registry, id.clone());

        let proposal = Blake2sHash::from([1u8; 32]);
        let vote_proposal = TendermintVote {
            proposal_hash: Some(proposal),
            id: id.clone(),
        };
        let vote_none = TendermintVote {
            proposal_hash: None,
            id,
        };

        // Slot 0 votes for the proposal, slot 1 votes for None: disjoint buckets.
        let proposal_bucket =
            TendermintContribution::from_vote(vote_proposal, &kp0.secret_key, 0..1);
        let none_bucket = TendermintContribution::from_vote(vote_none, &kp1.secret_key, 1..2);

        let mut disjoint = proposal_bucket;
        disjoint.contributions.extend(none_bucket.contributions);

        assert_eq!(block_on(verifier.verify(&disjoint)), VerificationResult::Ok,);
    }
}
