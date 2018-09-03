use beserial::{Serialize, Deserialize};
use consensus::base::transaction::Transaction;
use super::{Account, AccountError, AccountType};
use consensus::base::transaction::SignatureProof;
use consensus::base::primitive::hash::{Hasher, Blake2bHash, Blake2bHasher, HashAlgorithm};
use std::io;

#[derive(Clone, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
pub struct HashedTimeLockedContract {
    pub balance: u64
}

#[derive(Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum ProofType {
    RegularTransfer = 1,
    EarlyResolve = 2,
    TimeoutResolve = 3
}

impl HashedTimeLockedContract {
    pub fn create(balance: u64, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError> {
        unimplemented!();
    }

    pub fn verify_incoming_transaction(transaction: &Transaction, block_height: u32) -> bool {
        // The contract creation transaction is the only valid incoming transaction.
        return transaction.recipient == transaction.contract_creation_address();
    }

    pub fn verify_outgoing_transaction(transaction: &Transaction, block_height: u32) -> bool {

        fn bool_or_error(transaction: &Transaction, block_height: u32) -> io::Result<bool> {
            let transaction_content = transaction.serialize_content();
            let data = transaction_content.as_slice();
            let proof_buf = &mut &transaction.proof[..];

            match transaction.sender_type {
                AccountType::Basic => {
                    let signature_proof: SignatureProof = Deserialize::deserialize(proof_buf)?;
                    if signature_proof.compute_signer() != transaction.sender {
                        return Ok(false);
                    }
                    return Ok(signature_proof.public_key.verify(&signature_proof.signature, data));
                },
                AccountType::Vesting => {
                    let signature_proof: SignatureProof = Deserialize::deserialize(proof_buf)?;
                    return Ok(signature_proof.public_key.verify(&signature_proof.signature, data));
                },
                AccountType::HTLC => {
                    let proof_type: ProofType = Deserialize::deserialize(proof_buf)?;
                    match proof_type {
                        ProofType::RegularTransfer => {
                            let hash_algorithm: HashAlgorithm = Deserialize::deserialize(proof_buf)?;
                            let hash_depth: u8 = Deserialize::deserialize(proof_buf)?;
                            let hash_root: Blake2bHash = Deserialize::deserialize(proof_buf)?;
                            let mut pre_image: Blake2bHash = Deserialize::deserialize(proof_buf)?;

                            for i in 0..hash_depth {
                                match hash_algorithm {
                                    HashAlgorithm::Blake2b => {
                                        pre_image = Blake2bHasher::default().digest(pre_image.as_bytes());
                                    },
                                    _ => unimplemented!()
                                }
                            }
                            if hash_root != pre_image {
                                return Ok(false);
                            }
                            let signature_proof: SignatureProof = Deserialize::deserialize(proof_buf)?;
                            return Ok(signature_proof.public_key.verify(&signature_proof.signature, data));
                        },
                        ProofType::EarlyResolve => {
                            // Signature proof of the HTLC recipient
                            let signature_proof: SignatureProof = Deserialize::deserialize(proof_buf)?;
                            return Ok(signature_proof.public_key.verify(&signature_proof.signature, data) &&
                                signature_proof.public_key.verify(&signature_proof.signature, data));
                        },
                        ProofType::TimeoutResolve => {
                            // Signature proof of the HTLC creator
                            let signature_proof: SignatureProof = Deserialize::deserialize(proof_buf)?;
                            return Ok(signature_proof.public_key.verify(&signature_proof.signature, data));
                        }
                    };
                }
            };
        }
        match bool_or_error(transaction, block_height) {
            Ok(b) => return b,
            Err(e) => return false
        };
    }

    pub fn with_incoming_transaction(&self, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError> {
        return Err(AccountError("Illegal incoming transaction".to_string()));
    }

    pub fn without_incoming_transaction(&self, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError> {
        return Err(AccountError("Illegal incoming transaction".to_string()));
    }

    pub fn with_outgoing_transaction(&self, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError> {
        let balance: u64 = Account::balance_sub(self.balance, transaction.value + transaction.fee)?;
        return Ok(HashedTimeLockedContract { balance });
    }

    pub fn without_outgoing_transaction(&self, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError> {
        let balance: u64 = Account::balance_add(self.balance, transaction.value + transaction.fee)?;
        return Ok(HashedTimeLockedContract { balance });
    }
}
