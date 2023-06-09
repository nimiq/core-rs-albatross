use std::borrow::Cow;

use log::error;
use nimiq_hash::{Blake2bHasher, Hasher, Sha256Hasher};
use nimiq_keys::Address;
use nimiq_macros::{add_hex_io_fns_typed_arr, add_serialization_fns_typed_arr, create_typed_array};
use nimiq_primitives::account::AccountType;
use nimiq_serde::{Deserialize, Serialize};
use serde_repr::{Deserialize_repr, Serialize_repr};
use strum_macros::Display;

use crate::{
    account::AccountTransactionVerification, SignatureProof, Transaction, TransactionError,
    TransactionFlags,
};

/// The verifier trait for a hash time locked contract. This only uses data available in the transaction.
pub struct HashedTimeLockedContractVerifier {}

impl AccountTransactionVerification for HashedTimeLockedContractVerifier {
    fn verify_incoming_transaction(transaction: &Transaction) -> Result<(), TransactionError> {
        assert_eq!(transaction.recipient_type, AccountType::HTLC);

        if !transaction
            .flags
            .contains(TransactionFlags::CONTRACT_CREATION)
        {
            error!(
                "Only contract creation is allowed for the following transaction:\n{:?}",
                transaction
            );
            return Err(TransactionError::InvalidForRecipient);
        }

        if transaction.flags.contains(TransactionFlags::SIGNALING) {
            error!(
                "Signaling not allowed for the following transaction:\n{:?}",
                transaction
            );
            return Err(TransactionError::InvalidForRecipient);
        }

        if transaction.recipient != transaction.contract_creation_address() {
            warn!("Recipient address must match contract creation address for the following transaction:\n{:?}",
                transaction);
            return Err(TransactionError::InvalidForRecipient);
        }

        if transaction.data.len() != (20 * 2 + 1 + 32 + 1 + 8) {
            warn!(
                "Invalid data length. For the following transaction:\n{:?}",
                transaction
            );
            return Err(TransactionError::InvalidData);
        }

        CreationTransactionData::parse(transaction)?.verify()
    }

    fn verify_outgoing_transaction(transaction: &Transaction) -> Result<(), TransactionError> {
        assert_eq!(transaction.sender_type, AccountType::HTLC);

        let proof = OutgoingHTLCTransactionProof::parse(transaction)?;
        proof.verify(transaction)?;

        Ok(())
    }
}

#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Deserialize_repr,
    Display,
    Eq,
    Ord,
    PartialEq,
    PartialOrd,
    Serialize_repr,
    Hash,
)]
#[repr(u8)]
pub enum HashAlgorithm {
    #[default]
    Blake2b = 1,
    Sha256 = 3,
}

create_typed_array!(AnyHash, u8, 32);
add_hex_io_fns_typed_arr!(AnyHash, AnyHash::SIZE);
add_serialization_fns_typed_arr!(AnyHash, AnyHash::SIZE);

impl AnyHash {
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct CreationTransactionData {
    pub sender: Address,
    pub recipient: Address,
    pub hash_algorithm: HashAlgorithm,
    pub hash_root: AnyHash,
    pub hash_count: u8,
    #[serde(with = "nimiq_serde::fixint::be")]
    pub timeout: u64,
}

impl CreationTransactionData {
    pub fn parse(transaction: &Transaction) -> Result<Self, TransactionError> {
        Ok(Deserialize::deserialize_from_vec(&transaction.data[..])?)
    }

    pub fn verify(&self) -> Result<(), TransactionError> {
        if self.hash_count == 0 {
            warn!("Invalid creation data: hash_count may not be zero");
            return Err(TransactionError::InvalidData);
        }
        Ok(())
    }
}

/// The `OutgoingHTLCTransactionProof` represents a serializable form of all possible proof types
/// for a transaction from a HTLC contract.
///
/// The funds can be unlocked by one of three mechanisms:
/// 1. After a blockchain height called `timeout` is reached, the `sender` can withdraw the funds.
///     (called `TimeoutResolve`)
/// 2. The contract stores a `hash_root`. The `recipient` can withdraw the funds before the
///     `timeout` has been reached by presenting a hash that will yield the `hash_root`
///     when re-hashing it `hash_count` times.
///     By presenting a hash that will yield the `hash_root` after re-hashing it k < `hash_count`
///     times, the `recipient` can retrieve 1/k of the funds.
///     (called `RegularTransfer`)
/// 3. If both `sender` and `recipient` sign the transaction, the funds can be withdrawn at any time.
///     (called `EarlyResolve`)
#[derive(Clone, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum OutgoingHTLCTransactionProof {
    RegularTransfer {
        hash_algorithm: HashAlgorithm,
        hash_depth: u8,
        hash_root: AnyHash,
        pre_image: AnyHash,
        signature_proof: SignatureProof,
    },
    EarlyResolve {
        signature_proof_recipient: SignatureProof,
        signature_proof_sender: SignatureProof,
    },
    TimeoutResolve {
        signature_proof_sender: SignatureProof,
    },
}

impl OutgoingHTLCTransactionProof {
    pub fn parse(transaction: &Transaction) -> Result<Self, TransactionError> {
        let reader = &mut &transaction.proof[..];
        let (data, left_over) = Self::deserialize_take(reader)?;

        // Ensure that transaction data has been fully read.
        if !left_over.is_empty() {
            warn!("Over-long proof for the transaction");
            return Err(TransactionError::InvalidProof);
        }

        Ok(data)
    }

    pub fn verify(&self, transaction: &Transaction) -> Result<(), TransactionError> {
        // Verify proof.
        let tx_content = transaction.serialize_content();
        let tx_buf = tx_content.as_slice();

        match self {
            OutgoingHTLCTransactionProof::RegularTransfer {
                hash_algorithm,
                hash_depth,
                hash_root,
                pre_image,
                signature_proof,
            } => {
                let mut tmp_hash = pre_image.clone();
                for _ in 0..*hash_depth {
                    match hash_algorithm {
                        HashAlgorithm::Blake2b => {
                            tmp_hash = AnyHash(
                                Blake2bHasher::default().digest(tmp_hash.as_bytes()).into(),
                            );
                        }
                        HashAlgorithm::Sha256 => {
                            tmp_hash =
                                AnyHash(Sha256Hasher::default().digest(tmp_hash.as_bytes()).into());
                        }
                    }
                }

                if hash_root != &tmp_hash {
                    warn!(
                        "Hash algorithm mismatch for the following transaction:\n{:?}",
                        transaction
                    );
                    return Err(TransactionError::InvalidProof);
                }

                if !signature_proof.verify(tx_buf) {
                    warn!(
                        "Invalid signature for the following transaction:\n{:?}",
                        transaction
                    );
                    return Err(TransactionError::InvalidProof);
                }
            }
            OutgoingHTLCTransactionProof::EarlyResolve {
                signature_proof_recipient,
                signature_proof_sender,
            } => {
                if !signature_proof_recipient.verify(tx_buf)
                    || !signature_proof_sender.verify(tx_buf)
                {
                    warn!(
                        "Invalid signature for the following transaction:\n{:?}",
                        transaction
                    );
                    return Err(TransactionError::InvalidProof);
                }
            }
            OutgoingHTLCTransactionProof::TimeoutResolve {
                signature_proof_sender,
            } => {
                if !signature_proof_sender.verify(tx_buf) {
                    warn!(
                        "Invalid signature for the following transaction:\n{:?}",
                        transaction
                    );
                    return Err(TransactionError::InvalidProof);
                }
            }
        }

        Ok(())
    }
}

#[cfg(feature = "serde-derive")]
mod serde_derive {
    use std::borrow::Cow;

    use serde::{
        de::{Deserialize, Deserializer, Error},
        ser::{Serialize, Serializer},
    };

    use super::AnyHash;

    impl Serialize for AnyHash {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            serializer.serialize_str(&self.to_hex())
        }
    }

    impl<'de> Deserialize<'de> for AnyHash {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            let data: Cow<'de, str> = Deserialize::deserialize(deserializer)?;
            data.parse().map_err(Error::custom)
        }
    }
}
