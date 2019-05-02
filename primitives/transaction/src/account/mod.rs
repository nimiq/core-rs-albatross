use nimiq_bls::bls12_381::{PublicKey as BlsPublicKey, Signature as BlsSignature};
use nimiq_bls::Encoding;

use beserial::{Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};
use hash::{Blake2bHasher, Hasher, Sha256Hasher};
use keys::Address;
use primitives::account::{AccountType, AnyHash, HashAlgorithm, ProofType};
use primitives::coin::Coin;

use crate::{SignatureProof, Transaction, TransactionError, TransactionFlags};
use std::io::Read;

pub trait AccountTransactionVerification: Sized {
    fn verify_incoming_transaction(transaction: &Transaction) -> Result<(), TransactionError>;
    fn verify_outgoing_transaction(transaction: &Transaction) -> Result<(), TransactionError>;
}

impl AccountTransactionVerification for AccountType {
    fn verify_incoming_transaction(transaction: &Transaction) -> Result<(), TransactionError> {
        match transaction.recipient_type {
            AccountType::Basic => {
                // Do not allow the contract creation flag to be set for basic transactions.
                if transaction.flags.contains(TransactionFlags::CONTRACT_CREATION) {
                    return Err(TransactionError::InvalidForRecipient);
                }
                // Check that value > 0.
                if transaction.value == Coin::ZERO {
                    return Err(TransactionError::ZeroValue);
                }
                Ok(())
            },
            AccountType::HTLC => {
                // Check that value > 0.
                if transaction.value == Coin::ZERO {
                    return Err(TransactionError::ZeroValue);
                }
                parse_and_verify_htlc_creation_transaction(transaction)?;
                Ok(())
            },
            AccountType::Vesting => {
                // Check that value > 0.
                if transaction.value == Coin::ZERO {
                    return Err(TransactionError::ZeroValue);
                }
                parse_and_verify_vesting_creation_transaction(transaction)?;
                Ok(())
            },
            AccountType::Staking => {
                parse_and_verify_staking_transaction(transaction)?;
                Ok(())
            }
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
            },
            AccountType::Vesting | AccountType::Staking => {
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

    Ok((sender, recipient, hash_algorithm, hash_root, hash_count, timeout))
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
        Ok((owner, 0, vesting_step_blocks, transaction.value, transaction.value))
    } else if transaction.data.len() == Address::SIZE + 16 {
        let vesting_start = Deserialize::deserialize(reader)?;
        let vesting_step_blocks = Deserialize::deserialize(reader)?;
        let vesting_step_amount = Deserialize::deserialize(reader)?;
        Ok((owner, vesting_start, vesting_step_blocks, vesting_step_amount, transaction.value))
    } else if transaction.data.len() == Address::SIZE + 24 {
        // Create a vesting account with some instantly vested funds or additional funds considered.
        let vesting_start = Deserialize::deserialize(reader)?;
        let vesting_step_blocks = Deserialize::deserialize(reader)?;
        let vesting_step_amount = Deserialize::deserialize(reader)?;
        let vesting_total_amount = Deserialize::deserialize(reader)?;
        Ok((owner, vesting_start, vesting_step_blocks, vesting_step_amount, vesting_total_amount))
    } else {
        unreachable!();
    }
}

#[derive(Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum StakingTransactionType {
    Staking = 0,
    Restaking = 1,
    Pausing = 2,
}

pub enum StakingTransactionData {
    Staking {
        validator_key: BlsPublicKey,
        payout_address: Option<Address>,
        proof_of_knowledge: BlsSignature,
    },
    Restaking {
        old_validator_key: BlsPublicKey,
        new_validator_key: BlsPublicKey,
        proof_of_knowledge: BlsSignature,
        signature: BlsSignature,
    },
    Pausing {
        balance: Coin,
    },
}

impl StakingTransactionData {
    pub fn transaction_type(&self) -> StakingTransactionType {
        match self {
            StakingTransactionData::Staking {..} => StakingTransactionType::Staking,
            StakingTransactionData::Restaking {..} => StakingTransactionType::Restaking,
            StakingTransactionData::Pausing {..} => StakingTransactionType::Pausing,
        }
    }

    /// Important: Currently, the proof of knowledge of the secret key is a signature of the public key.
    /// If an attacker A ever tricks a validator B into signing a message with content `pk_A - pk_B`,
    /// where `pk_X` is X's BLS public key, A will be able to sign aggregate messages that are valid for
    /// public keys `pk_B + (pk_A - pk_B) = pk_B`.
    /// Alternatives would be to replace the proof of knowledge by a zero-knowledge proof.
    pub fn verify(&self) -> bool {
        match self {
            StakingTransactionData::Staking { validator_key, proof_of_knowledge, .. } => {
                validator_key.verify(&validator_key.to_bytes()[..], &proof_of_knowledge)
            },
            StakingTransactionData::Restaking {
                old_validator_key,
                new_validator_key,
                proof_of_knowledge,
                signature,
            } => {
                let mut content = old_validator_key.serialize_to_vec();
                content.append(&mut new_validator_key.serialize_to_vec());
                content.append(&mut proof_of_knowledge.serialize_to_vec());

                new_validator_key.verify(&new_validator_key.to_bytes()[..], &proof_of_knowledge) &&
                    old_validator_key.verify(content, signature) &&
                    old_validator_key != new_validator_key
            },
            StakingTransactionData::Pausing { .. } => {
                true
            },
        }
    }
}

impl Serialize for StakingTransactionData {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = self.transaction_type().serialize(writer)?;
        match self {
            StakingTransactionData::Staking { validator_key, payout_address, proof_of_knowledge } => {
                size += validator_key.serialize(writer)?;
                size += payout_address.serialize(writer)?;
                size += proof_of_knowledge.serialize(writer)?;
            },
            StakingTransactionData::Restaking {
                old_validator_key,
                new_validator_key,
                proof_of_knowledge,
                signature,
            } => {
                size += old_validator_key.serialize(writer)?;
                size += new_validator_key.serialize(writer)?;
                size += proof_of_knowledge.serialize(writer)?;
                size += signature.serialize(writer)?;
            },
            StakingTransactionData::Pausing { balance } => {
                size += balance.serialize(writer)?;
            },
        }
        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size = self.transaction_type().serialized_size();
        match self {
            StakingTransactionData::Staking { validator_key, payout_address, proof_of_knowledge } => {
                size += validator_key.serialized_size();
                size += payout_address.serialized_size();
                size += proof_of_knowledge.serialized_size();
            },
            StakingTransactionData::Restaking {
                old_validator_key,
                new_validator_key,
                proof_of_knowledge,
                signature,
            } => {
                size += old_validator_key.serialized_size();
                size += new_validator_key.serialized_size();
                size += proof_of_knowledge.serialized_size();
                size += signature.serialized_size();
            },
            StakingTransactionData::Pausing { balance } => {
                size += balance.serialized_size();
            },
        }
        size
    }
}

impl Deserialize for StakingTransactionData {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let ty: StakingTransactionType = Deserialize::deserialize(reader)?;
        match ty {
            StakingTransactionType::Staking => {
                let validator_key = Deserialize::deserialize(reader)?;
                let payout_address= Deserialize::deserialize(reader)?;
                let proof_of_knowledge = Deserialize::deserialize(reader)?;
                Ok(StakingTransactionData::Staking { validator_key, payout_address, proof_of_knowledge })
            },
            StakingTransactionType::Restaking => {
                let old_validator_key = Deserialize::deserialize(reader)?;
                let new_validator_key = Deserialize::deserialize(reader)?;
                let proof_of_knowledge = Deserialize::deserialize(reader)?;
                let signature = Deserialize::deserialize(reader)?;
                Ok(StakingTransactionData::Restaking {
                    old_validator_key,
                    new_validator_key,
                    proof_of_knowledge,
                    signature,
                })
            },
            StakingTransactionType::Pausing => {
                let balance = Deserialize::deserialize(reader)?;
                Ok(StakingTransactionData::Pausing { balance })
            },
        }
    }
}

pub fn parse_and_verify_staking_transaction(transaction: &Transaction) -> Result<StakingTransactionData, TransactionError> {
    assert_eq!(transaction.recipient_type, AccountType::Staking);

    if transaction.flags.contains(TransactionFlags::CONTRACT_CREATION) {
        return Err(TransactionError::InvalidForRecipient);
    }

    let data = parse_staking_transaction(transaction)?;

    // Staking transactions may not have a zero value!
    // Re-, and unstaking must have zero value!
    if (data.transaction_type() == StakingTransactionType::Staking) ==
        (transaction.value == Coin::ZERO) {
        return Err(TransactionError::ZeroValue);
    }

    if !data.verify() {
        return Err(TransactionError::InvalidData);
    }

    Ok(data)
}

pub fn parse_staking_transaction(transaction: &Transaction) -> Result<StakingTransactionData, TransactionError> {
    let reader = &mut &transaction.data[..];
    let data = Deserialize::deserialize(reader)?;

    // Ensure that everything is read.
    let mut other = Vec::new();
    reader.read_to_end(&mut other).map_err(|_| TransactionError::InvalidData)?;

    if !other.is_empty() {
        return Err(TransactionError::InvalidData);
    }

    Ok(data)
}
