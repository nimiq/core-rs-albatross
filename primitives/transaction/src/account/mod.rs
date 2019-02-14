use beserial::Deserialize;
use hash::{Blake2bHasher, Hasher, Sha256Hasher};
use keys::Address;
use primitives::account::{AccountType, AnyHash, HashAlgorithm, ProofType};
use primitives::coin::Coin;

use crate::{SignatureProof, Transaction, TransactionError, TransactionFlags};

pub trait AccountTransactionVerification: Sized {
    fn verify_incoming_transaction(transaction: &Transaction) -> Result<(), TransactionError>;
    fn verify_outgoing_transaction(transaction: &Transaction) -> Result<(), TransactionError>;
}

impl AccountTransactionVerification for AccountType {
    fn verify_incoming_transaction(transaction: &Transaction) -> Result<(), TransactionError> {
        match transaction.recipient_type {
            AccountType::Basic => {
                Ok(())
            },
            AccountType::HTLC => {
                parse_and_verify_htlc_creation_transaction(transaction)?;
                Ok(())
            },
            AccountType::Vesting => {
                parse_and_verify_vesting_creation_transaction(transaction)?;
                Ok(())
            },
        }
    }

    fn verify_outgoing_transaction(transaction: &Transaction) -> Result<(), TransactionError> {
        match transaction.sender_type {
            AccountType::Basic => {
                let signature_proof: SignatureProof = Deserialize::deserialize(&mut &transaction.proof[..])?;

                if !signature_proof.is_signed_by(&transaction.sender) || !signature_proof.verify(transaction.serialize_content().as_slice()) {
                    warn!("Invalid signature");
                    return Err(TransactionError::InvalidProof);
                }

                Ok(())
            },
            AccountType::HTLC => {
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

                        if proof_buf.len() != 0 {
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

                        if proof_buf.len() != 0 {
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

                        if proof_buf.len() != 0 {
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
            },
            AccountType::Vesting => {
                let signature_proof: SignatureProof = Deserialize::deserialize(&mut &transaction.proof[..])?;

                if !signature_proof.verify(transaction.serialize_content().as_slice()) {
                    warn!("Invalid signature");
                    return Err(TransactionError::InvalidProof);
                }

                Ok(())
            },
        }
    }
}

pub fn parse_and_verify_htlc_creation_transaction(transaction: &Transaction) -> Result<(Address, Address, HashAlgorithm, AnyHash, u8, u32), TransactionError> {
    assert_eq!(transaction.recipient_type, AccountType::HTLC);

    if !transaction.flags.contains(TransactionFlags::CONTRACT_CREATION) {
        warn!("Only contract creation is allowed");
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

    let (sender, recipient, hash_algorithm, hash_root, hash_count, timeout) = parse_htlc_creation_transaction(transaction)?;

    if hash_count == 0 {
        warn!("Invalid creation data: hash_count may not be zero");
        return Err(TransactionError::InvalidData);
    }

    Ok((sender, recipient, hash_algorithm, hash_root, hash_count, timeout))
}

pub fn parse_htlc_creation_transaction(transaction: &Transaction) -> Result<(Address, Address, HashAlgorithm, AnyHash, u8, u32), TransactionError> {
    let reader = &mut &transaction.data[..];

    let sender: Address = Deserialize::deserialize(reader)?;
    let recipient: Address = Deserialize::deserialize(reader)?;
    let hash_algorithm: HashAlgorithm = Deserialize::deserialize(reader)?;
    let hash_root = Deserialize::deserialize(reader)?;
    let hash_count = Deserialize::deserialize(reader)?;
    let timeout = Deserialize::deserialize(reader)?;

    return Ok((sender, recipient, hash_algorithm, hash_root, hash_count, timeout));
}

pub fn parse_and_verify_vesting_creation_transaction(transaction: &Transaction) -> Result<(Address, u32, u32, Coin, Coin), TransactionError> {
    assert_eq!(transaction.recipient_type, AccountType::Vesting);

    if !transaction.flags.contains(TransactionFlags::CONTRACT_CREATION) {
        warn!("Only contract creation is allowed");
        return Err(TransactionError::InvalidForRecipient);
    }

    // The contract creation transaction is the only valid incoming transaction.
    if transaction.recipient != transaction.contract_creation_address() {
        warn!("Only contract creation is allowed");
        return Err(TransactionError::InvalidForRecipient);
    }

    // Check that data field has the correct length.
    let allowed_sizes = [Address::SIZE + 4, Address::SIZE + 16, Address::SIZE + 24];
    if !allowed_sizes.contains(&transaction.data.len()) {
        warn!("Invalid creation data: invalid length");
        return Err(TransactionError::InvalidData);
    }

    Ok(parse_vesting_creation_transaction(transaction)?)
}

pub fn parse_vesting_creation_transaction(transaction: &Transaction) -> Result<(Address, u32, u32, Coin, Coin), TransactionError> {
    let reader = &mut &transaction.data[..];
    let owner = Deserialize::deserialize(reader)?;

    if transaction.data.len() == Address::SIZE + 4 {
        // Only block number: vest full amount at that block
        let vesting_step_blocks = Deserialize::deserialize(reader)?;
        return Ok((owner, 0, vesting_step_blocks, transaction.value, transaction.value));
    } else if transaction.data.len() == Address::SIZE + 16 {
        let vesting_start = Deserialize::deserialize(reader)?;
        let vesting_step_blocks = Deserialize::deserialize(reader)?;
        let vesting_step_amount = Deserialize::deserialize(reader)?;
        return Ok((owner, vesting_start, vesting_step_blocks, vesting_step_amount, transaction.value));
    } else if transaction.data.len() == Address::SIZE + 24 {
        // Create a vesting account with some instantly vested funds or additional funds considered.
        let vesting_start = Deserialize::deserialize(reader)?;
        let vesting_step_blocks = Deserialize::deserialize(reader)?;
        let vesting_step_amount = Deserialize::deserialize(reader)?;
        let vesting_total_amount = Deserialize::deserialize(reader)?;
        return Ok((owner, vesting_start, vesting_step_blocks, vesting_step_amount, vesting_total_amount));
    } else {
        unreachable!();
    }
}
