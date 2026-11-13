use std::sync::Arc;

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

        if contribution
            .proof
            .signature
            .verify_g1(&aggregated_public_key, self.hash_curve)
        {
            VerificationResult::Ok
        } else {
            VerificationResult::Forged
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nimiq_block::MultiSignature;
    use nimiq_bls::{AggregateSignature, KeyPair as BlsKeyPair};
    use nimiq_collections::BitSet;
    use nimiq_handel::verifier::{VerificationResult, Verifier};
    use nimiq_hash::Blake2sHash;
    use nimiq_keys::{Address, Ed25519PublicKey};
    use nimiq_primitives::slots_allocation::ValidatorsBuilder;
    use nimiq_test_log::test;
    use nimiq_test_utils::{bls::signature_not_on_curve, test_rng};
    use nimiq_utils::key_rng::SecureGenerate;

    use super::MultithreadedVerifier;
    use crate::aggregation::{registry::ValidatorRegistry, skip_block::SignedSkipBlockMessage};

    /// Builds a verifier over a registry with a single validator holding slot 0, along with that
    /// validator's valid contribution.
    fn setup() -> (
        MultithreadedVerifier<ValidatorRegistry>,
        SignedSkipBlockMessage,
    ) {
        let keypair = BlsKeyPair::generate(&mut test_rng(false));

        let mut builder = ValidatorsBuilder::new();
        builder.push(
            Address::START_ADDRESS,
            keypair.public_key,
            Ed25519PublicKey::default(),
        );
        let registry = Arc::new(ValidatorRegistry::new(builder.build()));

        let message_hash = Blake2sHash::from([1u8; 32]);
        let signature = AggregateSignature::from_signatures(&[keypair
            .secret_key
            .sign_hash(message_hash.clone())]);
        let message = SignedSkipBlockMessage {
            proof: MultiSignature::new(signature, BitSet::from_iter([0])),
        };

        (MultithreadedVerifier::new(message_hash, registry), message)
    }

    /// Control: a valid signature verifies, so the rejection below is caused by the invalid
    /// point.
    #[test(tokio::test)]
    async fn accepts_valid_signature() {
        let (verifier, message) = setup();

        assert_eq!(verifier.verify(&message).await, VerificationResult::Ok);
    }

    /// A signature that isn't a curve point must be rejected as forged.
    #[test(tokio::test)]
    async fn rejects_invalid_signature() {
        let (verifier, mut message) = setup();
        message.proof.signature = signature_not_on_curve().into();

        assert_eq!(verifier.verify(&message).await, VerificationResult::Forged);
    }
}
