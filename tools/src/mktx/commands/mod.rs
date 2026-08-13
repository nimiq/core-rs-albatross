pub mod basic;
pub mod htlc;
pub mod stake;
pub mod validator;
pub mod vesting;

use hex::FromHexError;
use nimiq_bls::{KeyPair as BlsKeyPair, SecretKey as BlsSecretKey};
use nimiq_hash::Blake2bHash;
use nimiq_keys::{Ed25519PublicKey, KeyPair, PrivateKey};
use nimiq_serde::{Deserialize, DeserializeError, Serialize};
use nimiq_transaction::{SignatureProof, Transaction};
use nimiq_transaction_builder::TransactionBuilderError;
use thiserror::Error;

pub enum TransactionOrProof {
    Transaction(Transaction),
    Proof(SignatureProof),
}

impl TransactionOrProof {
    pub fn to_hex(&self) -> String {
        match self {
            TransactionOrProof::Transaction(tx) => hex::encode(tx.serialize_to_vec()),
            TransactionOrProof::Proof(proof) => hex::encode(proof.serialize_to_vec()),
        }
    }
}

pub fn hex_secret_key_to_pair(secret_key: String) -> Result<KeyPair, CommandError> {
    Ok(PrivateKey::deserialize_from_vec(&hex::decode(secret_key)?)?.into())
}

pub fn hex_option_secret_key_to_option_pair(
    secret_key: Option<String>,
) -> Result<Option<KeyPair>, CommandError> {
    Ok(match secret_key {
        Some(secret_key) => Some(hex_secret_key_to_pair(secret_key)?),
        None => None,
    })
}

pub fn hex_to_signature_proof(signature_proof: String) -> Result<SignatureProof, CommandError> {
    Ok(SignatureProof::deserialize_from_vec(&hex::decode(
        signature_proof,
    )?)?)
}

pub fn hex_secret_key_to_public(secret_key: String) -> Result<Ed25519PublicKey, CommandError> {
    Ok(Ed25519PublicKey::from(&PrivateKey::deserialize_from_vec(
        &hex::decode(secret_key)?,
    )?))
}

pub fn hex_secret_key_to_bls_pair(secret_key: String) -> Result<BlsKeyPair, CommandError> {
    Ok(BlsKeyPair::from(BlsSecretKey::deserialize_from_vec(
        &hex::decode(secret_key)?,
    )?))
}

pub fn hex_to_signal_data(
    signal_data: Option<String>,
) -> Result<Option<Blake2bHash>, CommandError> {
    Ok(match signal_data {
        Some(signal_data) => Some(Blake2bHash::deserialize_from_vec(&hex::decode(
            signal_data,
        )?)?),
        None => None,
    })
}

#[derive(Debug, Error)]
pub enum CommandError {
    #[error("Deserialize")]
    Deserialize(#[from] DeserializeError),
    #[error("FromHex")]
    FromHex(#[from] FromHexError),
    #[error("TransactionBuilder")]
    TransactionBuilderError(#[from] TransactionBuilderError),
}
