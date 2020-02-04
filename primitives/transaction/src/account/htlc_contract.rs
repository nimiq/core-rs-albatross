use std::fmt::Display;

use enum_display_derive::Display;
use hex::FromHex;

use beserial::{Deserialize, Serialize};
use hash::{Blake2bHasher, Hasher, Sha256Hasher};
use keys::Address;
use macros::{add_hex_io_fns_typed_arr, create_typed_array};
use primitives::account::AccountType;
use primitives::coin::Coin;

use crate::{Transaction, TransactionError, TransactionFlags};
use crate::account::AccountTransactionVerification;
use crate::SignatureProof;

pub struct HashedTimeLockedContractVerifier {}

impl AccountTransactionVerification for HashedTimeLockedContractVerifier {
    fn verify_incoming_transaction(transaction: &Transaction) -> Result<(), TransactionError> {
        assert_eq!(transaction.recipient_type, AccountType::HTLC);

        if transaction.sender == transaction.recipient {
            return Err(TransactionError::SenderEqualsRecipient);
        }

        if !transaction.flags.contains(TransactionFlags::CONTRACT_CREATION) {
            warn!("Only contract creation is allowed");
            return Err(TransactionError::InvalidForRecipient);
        }

        if transaction.flags.contains(TransactionFlags::SIGNALLING) {
            warn!("Signalling not allowed");
            return Err(TransactionError::InvalidForRecipient);
        }

        if transaction.recipient != transaction.contract_creation_address() {
            warn!("Recipient address must match contract creation address");
            return Err(TransactionError::InvalidForRecipient);
        }

        if transaction.data.len() != (20 * 2 + 1 + 32 + 1 + 4) {
            warn!("Invalid creation data: invalid length");
            return Err(TransactionError::InvalidData);
        }

        CreationTransactionData::parse(transaction)?.verify()
    }

    fn verify_outgoing_transaction(transaction: &Transaction) -> Result<(), TransactionError> {
        assert_eq!(transaction.sender_type, AccountType::HTLC);

        // Verify proof.
        let tx_content = transaction.serialize_content();
        let tx_buf = tx_content.as_slice();

        let proof_buf = &mut &transaction.proof[..];
        let proof_type: ProofType = Deserialize::deserialize(proof_buf)?;
        match proof_type {
            ProofType::RegularTransfer => {
                let hash_algorithm: HashAlgorithm = Deserialize::deserialize(proof_buf)?;
                let hash_depth: u8 = Deserialize::deserialize(proof_buf)?;
                let hash_root: [u8; 32] = AnyHash::deserialize(proof_buf)?.into();
                let mut pre_image: [u8; 32] = AnyHash::deserialize(proof_buf)?.into();
                let signature_proof: SignatureProof = Deserialize::deserialize(proof_buf)?;

                if !proof_buf.is_empty() {
                    warn!("Over-long proof");
                    return Err(TransactionError::InvalidProof);
                }

                for _ in 0..hash_depth {
                    match hash_algorithm {
                        HashAlgorithm::Blake2b => {
                            pre_image = Blake2bHasher::default().digest(&pre_image[..]).into();
                        },
                        HashAlgorithm::Sha256 => {
                            pre_image = Sha256Hasher::default().digest(&pre_image[..]).into();
                        }
                    }
                }

                if hash_root != pre_image {
                    warn!("Hash mismatch");
                    return Err(TransactionError::InvalidProof);
                }

                if !signature_proof.verify(tx_buf) {
                    warn!("Invalid signature");
                    return Err(TransactionError::InvalidProof);
                }
            },
            ProofType::EarlyResolve => {
                let signature_proof_recipient: SignatureProof = Deserialize::deserialize(proof_buf)?;
                let signature_proof_sender: SignatureProof = Deserialize::deserialize(proof_buf)?;

                if !proof_buf.is_empty() {
                    warn!("Over-long proof");
                    return Err(TransactionError::InvalidProof)
                }

                if !signature_proof_recipient.verify(tx_buf) || !signature_proof_sender.verify(tx_buf) {
                    warn!("Invalid signature");
                    return Err(TransactionError::InvalidProof)
                }
            },
            ProofType::TimeoutResolve => {
                let signature_proof: SignatureProof = Deserialize::deserialize(proof_buf)?;

                if !proof_buf.is_empty() {
                    warn!("Over-long proof");
                    return Err(TransactionError::InvalidProof)
                }

                if !signature_proof.verify(tx_buf) {
                    warn!("Invalid signature");
                    return Err(TransactionError::InvalidProof)
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize, Display)]
#[repr(u8)]
pub enum HashAlgorithm {
    Blake2b = 1,
    Sha256 = 3
}

#[derive(Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum ProofType {
    RegularTransfer = 1,
    EarlyResolve = 2,
    TimeoutResolve = 3
}

create_typed_array!(AnyHash, u8, 32);
add_hex_io_fns_typed_arr!(AnyHash, AnyHash::SIZE);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreationTransactionData {
    pub sender: Address,
    pub recipient: Address,
    pub hash_algorithm: HashAlgorithm,
    pub hash_root: AnyHash,
    pub hash_count: u8,
    pub timeout: u32,
}

impl CreationTransactionData {
    pub fn parse(transaction: &Transaction) -> Result<Self, TransactionError> {
        Ok(Deserialize::deserialize(&mut &transaction.data[..])?)
    }

    pub fn verify(&self) -> Result<(), TransactionError> {
        if self.hash_count == 0 {
            warn!("Invalid creation data: hash_count may not be zero");
            return Err(TransactionError::InvalidData);
        }
        Ok(())
    }
}
