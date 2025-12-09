use nimiq_keys::{PublicKey, Signature};
use nimiq_serde::Deserialize;
use nimiq_transaction::account::htlc_contract::{
    AnyHash, CreationTransactionData, OutgoingHTLCTransactionProof, PoWCreationTransactionData,
    PoWOutgoingHTLCTransactionProof,
};
use wasm_bindgen::prelude::*;

use crate::common::transaction::{
    PlainHtlcData, PlainHtlcEarlyResolveProof, PlainHtlcRegularTransferProof,
    PlainHtlcTimeoutResolveProof, PlainTransactionProof, PlainTransactionRecipientData,
};
#[cfg(feature = "primitives")]
use crate::common::transaction::{PlainTransactionProofType, PlainTransactionRecipientDataType};

/// Utility class providing methods to parse Hashed Time Locked Contract transaction data and proofs.
#[wasm_bindgen]
pub struct HashedTimeLockedContract;

#[cfg(feature = "primitives")]
#[wasm_bindgen]
impl HashedTimeLockedContract {
    /// Parses the data of a Hashed Time Locked Contract creation transaction into a plain object.
    #[wasm_bindgen(js_name = dataToPlain)]
    pub fn data_to_plain(data: &[u8]) -> Result<PlainTransactionRecipientDataType, JsError> {
        let plain = HashedTimeLockedContract::parse_data(data, false, None, None)?;
        Ok(serde_wasm_bindgen::to_value(&plain)?.into())
    }

    /// Parses the proof of a Hashed Time Locked Contract settlement transaction into a plain object.
    #[wasm_bindgen(js_name = proofToPlain)]
    pub fn proof_to_plain(proof: &[u8]) -> Result<PlainTransactionProofType, JsError> {
        let plain = HashedTimeLockedContract::parse_proof(proof, false)?;
        Ok(serde_wasm_bindgen::to_value(&plain)?.into())
    }
}

impl HashedTimeLockedContract {
    pub fn parse_data(
        bytes: &[u8],
        as_pow: bool,
        genesis_block_number: Option<u32>,
        genesis_timestamp: Option<u64>,
    ) -> Result<PlainTransactionRecipientData, JsError> {
        let data = if as_pow {
            let genesis_block_number = genesis_block_number
                .ok_or_else(|| JsError::new("Genesis block number is required"))?;
            let genesis_timestamp =
                genesis_timestamp.ok_or_else(|| JsError::new("Genesis timestamp is required"))?;
            PoWCreationTransactionData::parse_data(bytes)?
                .into_pos(genesis_block_number, genesis_timestamp)
        } else {
            CreationTransactionData::parse_data(bytes)?
        };

        Ok(PlainTransactionRecipientData::Htlc(PlainHtlcData {
            raw: hex::encode(bytes),
            sender: data.sender.to_user_friendly_address(),
            recipient: data.recipient.to_user_friendly_address(),
            hash_algorithm: match data.hash_root {
                AnyHash::Blake2b(_) => "blake2b".to_string(),
                AnyHash::Sha256(_) => "sha256".to_string(),
                AnyHash::Sha512(_) => "sha512".to_string(),
                AnyHash::Keccak256(_) => "keccak256".to_string(),
            },
            hash_root: data.hash_root.to_hex(),
            hash_count: data.hash_count,
            timeout: data.timeout,
        }))
    }

    pub fn parse_proof(bytes: &[u8], as_pow: bool) -> Result<PlainTransactionProof, JsError> {
        let proof = if as_pow {
            PoWOutgoingHTLCTransactionProof::deserialize_all(bytes)?.into_pos()
        } else {
            OutgoingHTLCTransactionProof::deserialize_all(bytes)?
        };

        Ok(match proof {
            OutgoingHTLCTransactionProof::RegularTransfer {
                hash_depth,
                hash_root,
                pre_image,
                signature_proof,
            } => PlainTransactionProof::RegularTransfer(PlainHtlcRegularTransferProof {
                raw: hex::encode(bytes),
                hash_algorithm: match hash_root {
                    AnyHash::Blake2b(_) => "blake2b".to_string(),
                    AnyHash::Sha256(_) => "sha256".to_string(),
                    AnyHash::Sha512(_) => "sha512".to_string(),
                    AnyHash::Keccak256(_) => "keccak256".to_string(),
                },
                hash_depth,
                hash_root: hash_root.to_hex(),
                pre_image: pre_image.to_hex(),
                signer: signature_proof.compute_signer().to_user_friendly_address(),
                signature: match signature_proof.signature {
                    Signature::Ed25519(ref signature) => signature.to_hex(),
                    Signature::ES256(ref signature) => signature.to_hex(),
                },
                public_key: match signature_proof.public_key {
                    PublicKey::Ed25519(ref public_key) => public_key.to_hex(),
                    PublicKey::ES256(ref public_key) => public_key.to_hex(),
                },
                path_length: signature_proof.merkle_path.len() as u8,
            }),
            OutgoingHTLCTransactionProof::TimeoutResolve {
                signature_proof_sender,
            } => PlainTransactionProof::TimeoutResolve(PlainHtlcTimeoutResolveProof {
                raw: hex::encode(bytes),
                creator: signature_proof_sender
                    .compute_signer()
                    .to_user_friendly_address(),
                creator_signature: match signature_proof_sender.signature {
                    Signature::Ed25519(ref signature) => signature.to_hex(),
                    Signature::ES256(ref signature) => signature.to_hex(),
                },
                creator_public_key: match signature_proof_sender.public_key {
                    PublicKey::Ed25519(ref public_key) => public_key.to_hex(),
                    PublicKey::ES256(ref public_key) => public_key.to_hex(),
                },
                creator_path_length: signature_proof_sender.merkle_path.len() as u8,
            }),
            OutgoingHTLCTransactionProof::EarlyResolve {
                signature_proof_recipient,
                signature_proof_sender,
            } => PlainTransactionProof::EarlyResolve(PlainHtlcEarlyResolveProof {
                raw: hex::encode(bytes),
                signer: signature_proof_recipient
                    .compute_signer()
                    .to_user_friendly_address(),
                signature: match signature_proof_recipient.signature {
                    Signature::Ed25519(ref signature) => signature.to_hex(),
                    Signature::ES256(ref signature) => signature.to_hex(),
                },
                public_key: match signature_proof_recipient.public_key {
                    PublicKey::Ed25519(ref public_key) => public_key.to_hex(),
                    PublicKey::ES256(ref public_key) => public_key.to_hex(),
                },
                path_length: signature_proof_recipient.merkle_path.len() as u8,
                creator: signature_proof_sender
                    .compute_signer()
                    .to_user_friendly_address(),
                creator_signature: match signature_proof_sender.signature {
                    Signature::Ed25519(ref signature) => signature.to_hex(),
                    Signature::ES256(ref signature) => signature.to_hex(),
                },
                creator_public_key: match signature_proof_sender.public_key {
                    PublicKey::Ed25519(ref public_key) => public_key.to_hex(),
                    PublicKey::ES256(ref public_key) => public_key.to_hex(),
                },
                creator_path_length: signature_proof_sender.merkle_path.len() as u8,
            }),
        })
    }
}
