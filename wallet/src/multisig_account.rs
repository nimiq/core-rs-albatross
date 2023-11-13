use std::num::NonZeroU8;

use nimiq_hash::Blake2bHasher;
use nimiq_keys::{
    multisig::{
        address::{combine_public_keys, compute_address},
        commitment::{Commitment, CommitmentPair},
        error::PartialSignatureError,
        partial_signature::PartialSignature,
        public_key::DelinearizedPublicKey,
        CommitmentsData, MUSIG2_PARAMETER_V,
    },
    Address, EdDSAPublicKey, KeyPair, SecureGenerate,
};
use nimiq_primitives::{coin::Coin, networks::NetworkId};
use nimiq_serde::Serialize;
use nimiq_transaction::{EdDSASignatureProof, Transaction};
use nimiq_utils::merkle::Blake2bMerklePath;
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
    pub public_keys: Vec<EdDSAPublicKey>,
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
        public_keys: &[EdDSAPublicKey],
    ) -> Result<Self, MultiSigAccountError> {
        if public_keys.is_empty() {
            return Err(MultiSigAccountError::PublicKeysNotEmpty);
        } else if !public_keys.contains(&key_pair.public) {
            return Err(MultiSigAccountError::KeyPairNotPartOfList);
        }

        let mut sorted_public_keys = public_keys.to_vec();
        sorted_public_keys.sort();

        let multi_sig_keys = combine_public_keys(sorted_public_keys, min_signatures.get() as usize);

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
        public_keys: &[EdDSAPublicKey],
    ) -> Self {
        Self {
            address: compute_address(public_keys),
            key_pair: key_pair.clone(),
            min_signatures,
            public_keys: public_keys.to_vec(),
        }
    }

    /// Generates a new commitment pair containing a commitment and a random secret.
    #[inline]
    pub fn create_commitments(&self) -> [CommitmentPair; MUSIG2_PARAMETER_V] {
        let mut own_commitments = Vec::with_capacity(MUSIG2_PARAMETER_V);
        for _ in 0..MUSIG2_PARAMETER_V {
            own_commitments.push(CommitmentPair::generate_default_csprng());
        }
        own_commitments.try_into().unwrap()
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
    pub fn aggregate_public_keys(public_keys: &[EdDSAPublicKey]) -> EdDSAPublicKey {
        DelinearizedPublicKey::sum_delinearized(public_keys)
    }

    /// Creates a partial signature of the provided transaction.
    pub fn partially_sign_transaction(
        &self,
        transaction: &Transaction,
        commitments_data: &CommitmentsData,
    ) -> Result<PartialSignature, PartialSignatureError> {
        self.key_pair
            .partial_sign(commitments_data, transaction.serialize_content().as_slice())
    }

    /// Creates a signature proof.
    pub fn create_proof(
        &self,
        aggregated_public_key: &EdDSAPublicKey,
        aggregated_commitment: &Commitment,
        partial_signatures: &[PartialSignature],
    ) -> Result<EdDSASignatureProof, MultiSigAccountError> {
        if partial_signatures.len() != self.min_signatures.get() as usize {
            return Err(MultiSigAccountError::InvalidSignaturesLength);
        }

        let aggregated_signature: PartialSignature = partial_signatures.iter().sum();
        let signature = aggregated_signature.to_signature(aggregated_commitment);

        Ok(EdDSASignatureProof {
            merkle_path: Blake2bMerklePath::new::<Blake2bHasher, _>(
                &self.public_keys,
                aggregated_public_key,
            ),
            public_key: *aggregated_public_key,
            signature,
        })
    }

    /// Signs the transaction.
    pub fn sign_transaction(
        &self,
        transaction: &Transaction,
        aggregated_public_key: &EdDSAPublicKey,
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
