use std::num::NonZeroU8;

use curve25519_dalek::EdwardsPoint;
use itertools::Itertools;
use nimiq_hash::Blake2bHasher;
use nimiq_keys::{
    multisig::{hash_public_keys, Commitment, CommitmentPair, PartialSignature, RandomSecret},
    Address, Ed25519PublicKey, Ed25519Signature, KeyPair, SecureGenerate, SecureRng,
};
use nimiq_primitives::{coin::Coin, networks::NetworkId};
use nimiq_serde::Serialize;
use nimiq_transaction::{SignatureProof, Transaction};
use nimiq_utils::merkle::{compute_root_from_content_slice, MerklePath};
use thiserror::Error;

/// A multi-signature account is an account that requires multiple signatures to authorize outgoing transactions.
#[derive(Debug)]
pub struct MultiSigAccount {
    /// The public address that is used to interact with other accounts.
    pub address: Address,
    /// The keypair owning this account.
    pub key_pair: KeyPair,
    /// Minimum number of required signatures.
    pub min_signatures: NonZeroU8,
    /// A list of all aggregated public keys.
    pub public_keys: Vec<Ed25519PublicKey>,
}

impl MultiSigAccount {
    /// Returns a new MultiSignature Account.
    ///
    /// # Arguments
    ///
    /// * `key_pair` - Keypair owning this account.
    /// * `min_signatures` - Number of signatures required.
    /// * `public_keys` - A list of all owners' public keys. The public key of the `key_pair` must at least be one of the elements.
    pub fn from_public_keys(
        key_pair: &KeyPair,
        min_signatures: NonZeroU8,
        public_keys: &[Ed25519PublicKey],
    ) -> Result<Self, MultiSigAccountError> {
        if public_keys.is_empty() {
            return Err(MultiSigAccountError::PublicKeysNotEmpty);
        } else if !public_keys.contains(&key_pair.public) {
            return Err(MultiSigAccountError::KeyPairNotPartOfList);
        }

        let mut sorted_public_keys = public_keys.to_vec();
        sorted_public_keys.sort();

        let multi_sig_keys: Vec<Ed25519PublicKey> = sorted_public_keys
            .into_iter()
            .combinations(min_signatures.get() as usize)
            .map(|pk| MultiSigAccount::aggregate_public_keys(&pk))
            .collect();

        Ok(Self::new(key_pair, min_signatures, &multi_sig_keys))
    }

    /// Returns a new MultiSignature Account. This method expects that the provided public keys are already delinearized and aggregated.
    /// Use `MultiSigAccount.from_public_keys` otherwise.
    ///
    /// # Arguments
    ///
    /// * `key_pair` - Keypair owning this account.
    /// * `min_signatures` - Number of signatures required.
    /// * `public_keys` - A list of all aggregated public keys.
    pub fn new(
        key_pair: &KeyPair,
        min_signatures: NonZeroU8,
        public_keys: &[Ed25519PublicKey],
    ) -> Self {
        Self {
            address: Address::from(compute_root_from_content_slice::<
                Blake2bHasher,
                Ed25519PublicKey,
            >(public_keys)),
            key_pair: key_pair.clone(),
            min_signatures,
            public_keys: public_keys.to_vec(),
        }
    }

    /// Generates a new commitment pair containing a commitment and a random secret.
    #[inline]
    pub fn create_commitment(&self) -> CommitmentPair {
        CommitmentPair::generate(&mut SecureRng::default())
    }

    /// Creates an unsigned transaction.
    pub fn create_transaction(
        &self,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: u32,
        network_id: NetworkId,
    ) -> Transaction {
        Transaction::new_basic(
            self.address.clone(),
            recipient,
            value,
            fee,
            validity_start_height,
            network_id,
        )
    }

    /// Utility method that delinearizes and aggregates the provided slice of public keys.
    pub fn aggregate_public_keys(public_keys: &[Ed25519PublicKey]) -> Ed25519PublicKey {
        let mut sorted_public_keys = public_keys.to_vec();
        sorted_public_keys.sort();

        let public_keys_hash = hash_public_keys(&sorted_public_keys);
        let delinearized_pk_sum: EdwardsPoint = public_keys
            .iter()
            .map(|public_key| public_key.delinearize(&public_keys_hash))
            .sum();

        let mut public_key_bytes: [u8; Ed25519PublicKey::SIZE] = [0u8; Ed25519PublicKey::SIZE];
        public_key_bytes.copy_from_slice(delinearized_pk_sum.compress().as_bytes());
        Ed25519PublicKey::from(public_key_bytes)
    }

    /// Creates a partial signature of the provided transaction.
    pub fn partially_sign_transaction(
        &self,
        transaction: &Transaction,
        public_keys: &[Ed25519PublicKey],
        commitments: &[Commitment],
        secret: &RandomSecret,
    ) -> PartialSignature {
        let mut sorted_public_keys = public_keys.to_vec();
        sorted_public_keys.sort();

        self.key_pair
            .partial_sign(
                &sorted_public_keys,
                secret,
                commitments,
                transaction.serialize_content().as_slice(),
            )
            .0
    }

    /// Creates a signature proof.
    pub fn create_proof(
        &self,
        aggregated_public_key: &Ed25519PublicKey,
        aggregated_commitment: &Commitment,
        signatures: &[PartialSignature],
    ) -> Result<SignatureProof, MultiSigAccountError> {
        if signatures.len() != self.min_signatures.get() as usize {
            return Err(MultiSigAccountError::InvalidSignaturesLength);
        }

        let aggregated_signature: PartialSignature = signatures.iter().sum();
        let raw_aggregated_signature = aggregated_signature.as_bytes();
        let raw_aggregated_commitment = aggregated_commitment.to_bytes();

        let mut combined =
            Vec::with_capacity(raw_aggregated_signature.len() + raw_aggregated_commitment.len());
        combined.extend_from_slice(&aggregated_commitment.to_bytes());
        combined.extend_from_slice(aggregated_signature.as_bytes());
        let signature = Ed25519Signature::from_bytes(&combined)?;

        let mut proof = SignatureProof::from_ed25519(*aggregated_public_key, signature);
        proof.merkle_path = MerklePath::new::<Blake2bHasher, Ed25519PublicKey>(
            &self.public_keys,
            aggregated_public_key,
        );
        Ok(proof)
    }

    /// Signs the transaction.
    pub fn sign_transaction(
        &self,
        transaction: &Transaction,
        aggregated_public_key: &Ed25519PublicKey,
        aggregated_commitment: &Commitment,
        partial_signatures: &[PartialSignature],
    ) -> Result<Transaction, MultiSigAccountError> {
        let proof = self.create_proof(
            aggregated_public_key,
            aggregated_commitment,
            partial_signatures,
        )?;

        let mut signed_transaction = transaction.clone();
        signed_transaction.proof = proof.serialize_to_vec();

        Ok(signed_transaction)
    }
}

/// Possible multi-sig account errors.
#[derive(Debug, Error)]
pub enum MultiSigAccountError {
    #[error("Invalid signature constructed")]
    InvalidSignatureConstructed,
    #[error("Failed to construct a valid signature based on the provided signatures and aggregated commitment")]
    InvalidSignatureFromBytes(#[from] nimiq_keys::SignatureError),
    #[error("Number of signatures must be the same as the minimal signatures")]
    InvalidSignaturesLength,
    #[error("The public key of keypair must be part of provided public keys")]
    KeyPairNotPartOfList,
    #[error("The provided public keys must not be empty")]
    PublicKeysNotEmpty,
}
