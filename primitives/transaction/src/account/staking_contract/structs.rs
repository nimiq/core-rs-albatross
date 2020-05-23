use beserial::{Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};
use bls::{CompressedPublicKey as BlsPublicKey, CompressedSignature as BlsSignature};
use keys::Address;

use crate::SignatureProof;
use crate::{AccountType, Transaction, TransactionError};

/// We need to distinguish three types of transactions:
/// 1. Incoming transactions, which include:
///     - Validator
///         * Create
///         * Update
///         * Retire
///         * Re-activate
///         * Unpark
///     - Staker
///         * Stake
///     The type of transaction is given in the data field.
/// 2. Outgoing transactions, which include:
///     - Validator
///         * Drop
///     - Staker
///         * Unstake
///     The type of transaction is given in the proof field.
/// 3. Self transactions, which include:
///     - Staker
///         * Retire
///         * Re-activate
///     The type of transaction is given in the data field.
///     The signature proof is given in the proof field.
///
#[derive(Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum IncomingStakingTransactionType {
    CreateValidator = 0,
    UpdateValidator = 1,
    RetireValidator = 2,
    ReactivateValidator = 3,
    UnparkValidator = 4,
    Stake = 5,
}

impl IncomingStakingTransactionType {
    pub fn is_signalling(&self) -> bool {
        match self {
            IncomingStakingTransactionType::Stake
            | IncomingStakingTransactionType::CreateValidator => false,
            _ => true,
        }
    }
}

/// It is important to note that all `signature` fields contain the signature
/// over the complete transaction with the `signature` field set to `Default::default()`.
/// The field is populated only after computing the signature.
#[derive(Clone, Debug)]
pub enum IncomingStakingTransactionData {
    CreateValidator {
        validator_key: BlsPublicKey,
        proof_of_knowledge: BlsSignature,
        reward_address: Address,
    },
    UpdateValidator {
        old_validator_key: BlsPublicKey,
        new_validator_key: Option<BlsPublicKey>,
        new_proof_of_knowledge: Option<BlsSignature>,
        new_reward_address: Option<Address>,
        signature: BlsSignature,
    },
    RetireValidator {
        validator_key: BlsPublicKey,
        signature: BlsSignature,
    },
    ReactivateValidator {
        validator_key: BlsPublicKey,
        signature: BlsSignature,
    },
    UnparkValidator {
        validator_key: BlsPublicKey,
        signature: BlsSignature,
    },
    Stake {
        validator_key: BlsPublicKey,
        // If no staker_address is given, the sender's address is used.
        staker_address: Option<Address>,
    },
}

impl IncomingStakingTransactionData {
    pub fn is_signalling(&self) -> bool {
        match self {
            IncomingStakingTransactionData::Stake { .. }
            | IncomingStakingTransactionData::CreateValidator { .. } => false,
            _ => true,
        }
    }

    pub fn parse(transaction: &Transaction) -> Result<Self, TransactionError> {
        full_parse(&transaction.data[..])
    }

    pub fn verify(&self, transaction: &Transaction) -> Result<(), TransactionError> {
        match self {
            IncomingStakingTransactionData::CreateValidator {
                validator_key,
                proof_of_knowledge,
                ..
            } => {
                // Check proof of knowledge.
                verify_proof_of_knowledge(validator_key, proof_of_knowledge)?;
            }
            IncomingStakingTransactionData::UpdateValidator {
                old_validator_key,
                new_validator_key,
                new_proof_of_knowledge,
                signature,
                new_reward_address,
            } => {
                // Check signature and proof of knowledge.
                verify_transaction_signature(transaction, old_validator_key, signature)?;

                // Do not allow updates without any effect.
                if new_validator_key.is_none() && new_reward_address.is_none() {
                    return Err(TransactionError::InvalidData);
                }

                if let (Some(new_validator_key), Some(new_proof_of_knowledge)) =
                    (new_validator_key, new_proof_of_knowledge)
                {
                    verify_proof_of_knowledge(new_validator_key, new_proof_of_knowledge)?;
                }
            }
            IncomingStakingTransactionData::RetireValidator {
                validator_key,
                signature,
            } => {
                // Check signature.
                verify_transaction_signature(transaction, validator_key, signature)?;
            }
            IncomingStakingTransactionData::ReactivateValidator {
                validator_key,
                signature,
            } => {
                // Check signature.
                verify_transaction_signature(transaction, validator_key, signature)?;
            }
            IncomingStakingTransactionData::UnparkValidator {
                validator_key,
                signature,
            } => {
                // Check signature.
                verify_transaction_signature(transaction, validator_key, signature)?;
            }
            IncomingStakingTransactionData::Stake { .. } => {
                // No checks required.
            }
        }
        Ok(())
    }

    pub fn set_validator_signature(&mut self, validator_signature: BlsSignature) {
        match self {
            IncomingStakingTransactionData::UpdateValidator { signature, .. } => {
                *signature = validator_signature;
            }
            IncomingStakingTransactionData::RetireValidator { signature, .. } => {
                *signature = validator_signature;
            }
            IncomingStakingTransactionData::UnparkValidator { signature, .. } => {
                *signature = validator_signature;
            }
            IncomingStakingTransactionData::ReactivateValidator { signature, .. } => {
                *signature = validator_signature;
            }
            _ => {}
        }
    }

    pub fn set_validator_signature_on_data(
        data: &[u8],
        validator_signature: BlsSignature,
    ) -> Result<Vec<u8>, SerializingError> {
        let mut data: IncomingStakingTransactionData = Deserialize::deserialize_from_vec(data)?;
        data.set_validator_signature(validator_signature);
        Ok(data.serialize_to_vec())
    }
}

impl Serialize for IncomingStakingTransactionData {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = 0;
        match self {
            IncomingStakingTransactionData::CreateValidator {
                validator_key,
                proof_of_knowledge,
                reward_address,
            } => {
                size +=
                    Serialize::serialize(&IncomingStakingTransactionType::CreateValidator, writer)?;
                size += Serialize::serialize(validator_key, writer)?;
                size += Serialize::serialize(proof_of_knowledge, writer)?;
                size += Serialize::serialize(reward_address, writer)?;
            }
            IncomingStakingTransactionData::UpdateValidator {
                old_validator_key,
                new_validator_key,
                new_proof_of_knowledge,
                new_reward_address,
                signature,
            } => {
                size +=
                    Serialize::serialize(&IncomingStakingTransactionType::UpdateValidator, writer)?;
                size += Serialize::serialize(old_validator_key, writer)?;
                size += Serialize::serialize(&new_validator_key.is_some(), writer)?;
                size += Serialize::serialize(&new_reward_address.is_some(), writer)?;
                if let (Some(new_validator_key), Some(new_proof_of_knowledge)) =
                    (new_validator_key, new_proof_of_knowledge)
                {
                    size += Serialize::serialize(new_validator_key, writer)?;
                    size += Serialize::serialize(new_proof_of_knowledge, writer)?;
                }
                if let Some(new_reward_address) = new_reward_address {
                    size += Serialize::serialize(new_reward_address, writer)?;
                }
                size += Serialize::serialize(signature, writer)?;
            }
            IncomingStakingTransactionData::RetireValidator {
                validator_key,
                signature,
            } => {
                size +=
                    Serialize::serialize(&IncomingStakingTransactionType::RetireValidator, writer)?;
                size += Serialize::serialize(validator_key, writer)?;
                size += Serialize::serialize(signature, writer)?;
            }
            IncomingStakingTransactionData::ReactivateValidator {
                validator_key,
                signature,
            } => {
                size += Serialize::serialize(
                    &IncomingStakingTransactionType::ReactivateValidator,
                    writer,
                )?;
                size += Serialize::serialize(validator_key, writer)?;
                size += Serialize::serialize(signature, writer)?;
            }
            IncomingStakingTransactionData::UnparkValidator {
                validator_key,
                signature,
            } => {
                size +=
                    Serialize::serialize(&IncomingStakingTransactionType::UnparkValidator, writer)?;
                size += Serialize::serialize(validator_key, writer)?;
                size += Serialize::serialize(signature, writer)?;
            }
            IncomingStakingTransactionData::Stake {
                validator_key,
                staker_address,
            } => {
                size += Serialize::serialize(&IncomingStakingTransactionType::Stake, writer)?;
                size += Serialize::serialize(validator_key, writer)?;
                size += Serialize::serialize(staker_address, writer)?;
            }
        }
        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size = 0;
        match self {
            IncomingStakingTransactionData::CreateValidator {
                validator_key,
                proof_of_knowledge,
                reward_address,
            } => {
                size +=
                    Serialize::serialized_size(&IncomingStakingTransactionType::CreateValidator);
                size += Serialize::serialized_size(validator_key);
                size += Serialize::serialized_size(proof_of_knowledge);
                size += Serialize::serialized_size(reward_address);
            }
            IncomingStakingTransactionData::UpdateValidator {
                old_validator_key,
                new_validator_key,
                new_proof_of_knowledge,
                new_reward_address,
                signature,
            } => {
                size +=
                    Serialize::serialized_size(&IncomingStakingTransactionType::UpdateValidator);
                size += Serialize::serialized_size(old_validator_key);
                size += Serialize::serialized_size(&new_validator_key.is_some());
                size += Serialize::serialized_size(&new_reward_address.is_some());
                if let (Some(new_validator_key), Some(new_proof_of_knowledge)) =
                    (new_validator_key, new_proof_of_knowledge)
                {
                    size += Serialize::serialized_size(new_validator_key);
                    size += Serialize::serialized_size(new_proof_of_knowledge);
                }
                if let Some(new_reward_address) = new_reward_address {
                    size += Serialize::serialized_size(new_reward_address);
                }
                size += Serialize::serialized_size(signature);
            }
            IncomingStakingTransactionData::RetireValidator {
                validator_key,
                signature,
            } => {
                size +=
                    Serialize::serialized_size(&IncomingStakingTransactionType::RetireValidator);
                size += Serialize::serialized_size(validator_key);
                size += Serialize::serialized_size(signature);
            }
            IncomingStakingTransactionData::ReactivateValidator {
                validator_key,
                signature,
            } => {
                size += Serialize::serialized_size(
                    &IncomingStakingTransactionType::ReactivateValidator,
                );
                size += Serialize::serialized_size(validator_key);
                size += Serialize::serialized_size(signature);
            }
            IncomingStakingTransactionData::UnparkValidator {
                validator_key,
                signature,
            } => {
                size +=
                    Serialize::serialized_size(&IncomingStakingTransactionType::UnparkValidator);
                size += Serialize::serialized_size(validator_key);
                size += Serialize::serialized_size(signature);
            }
            IncomingStakingTransactionData::Stake {
                validator_key,
                staker_address,
            } => {
                size += Serialize::serialized_size(&IncomingStakingTransactionType::Stake);
                size += Serialize::serialized_size(validator_key);
                size += Serialize::serialized_size(staker_address);
            }
        }
        size
    }
}

impl Deserialize for IncomingStakingTransactionData {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let ty: IncomingStakingTransactionType = Deserialize::deserialize(reader)?;
        match ty {
            IncomingStakingTransactionType::CreateValidator => {
                Ok(IncomingStakingTransactionData::CreateValidator {
                    validator_key: Deserialize::deserialize(reader)?,
                    proof_of_knowledge: Deserialize::deserialize(reader)?,
                    reward_address: Deserialize::deserialize(reader)?,
                })
            }
            IncomingStakingTransactionType::UpdateValidator => {
                let old_validator_key: BlsPublicKey = Deserialize::deserialize(reader)?;
                let updates_key: bool = Deserialize::deserialize(reader)?;
                let updates_address: bool = Deserialize::deserialize(reader)?;

                let mut new_validator_key = None;
                let mut new_proof_of_knowledge = None;
                let mut new_reward_address = None;
                if updates_key {
                    new_validator_key = Some(Deserialize::deserialize(reader)?);
                    new_proof_of_knowledge = Some(Deserialize::deserialize(reader)?);
                }
                if updates_address {
                    new_reward_address = Some(Deserialize::deserialize(reader)?);
                }
                let signature = Deserialize::deserialize(reader)?;

                Ok(IncomingStakingTransactionData::UpdateValidator {
                    old_validator_key,
                    new_validator_key,
                    new_proof_of_knowledge,
                    new_reward_address,
                    signature,
                })
            }
            IncomingStakingTransactionType::RetireValidator => {
                let validator_key = Deserialize::deserialize(reader)?;
                let signature = Deserialize::deserialize(reader)?;
                Ok(IncomingStakingTransactionData::RetireValidator {
                    validator_key,
                    signature,
                })
            }
            IncomingStakingTransactionType::ReactivateValidator => {
                Ok(IncomingStakingTransactionData::ReactivateValidator {
                    validator_key: Deserialize::deserialize(reader)?,
                    signature: Deserialize::deserialize(reader)?,
                })
            }
            IncomingStakingTransactionType::UnparkValidator => {
                Ok(IncomingStakingTransactionData::UnparkValidator {
                    validator_key: Deserialize::deserialize(reader)?,
                    signature: Deserialize::deserialize(reader)?,
                })
            }
            IncomingStakingTransactionType::Stake => Ok(IncomingStakingTransactionData::Stake {
                validator_key: Deserialize::deserialize(reader)?,
                staker_address: Deserialize::deserialize(reader)?,
            }),
        }
    }
}

#[derive(Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum OutgoingStakingTransactionType {
    DropValidator = 0,
    Unstake = 5,
}

#[derive(Clone, Debug)]
pub enum OutgoingStakingTransactionProof {
    DropValidator {
        validator_key: BlsPublicKey,
        signature: BlsSignature,
    },
    Unstake(SignatureProof),
}

impl OutgoingStakingTransactionProof {
    pub fn parse(transaction: &Transaction) -> Result<Self, TransactionError> {
        full_parse(&transaction.proof[..])
    }

    pub fn verify(&self, transaction: &Transaction) -> Result<(), TransactionError> {
        match self {
            OutgoingStakingTransactionProof::DropValidator {
                validator_key,
                signature,
            } => verify_transaction_signature(transaction, validator_key, signature),
            OutgoingStakingTransactionProof::Unstake(signature_proof) => {
                if !signature_proof.verify(transaction.serialize_content().as_slice()) {
                    warn!("Invalid signature");
                    return Err(TransactionError::InvalidProof);
                }
                Ok(())
            }
        }
    }
}

impl Serialize for OutgoingStakingTransactionProof {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = 0;
        match self {
            OutgoingStakingTransactionProof::DropValidator {
                validator_key,
                signature,
            } => {
                size +=
                    Serialize::serialize(&OutgoingStakingTransactionType::DropValidator, writer)?;
                size += Serialize::serialize(validator_key, writer)?;
                size += Serialize::serialize(signature, writer)?;
            }
            OutgoingStakingTransactionProof::Unstake(proof) => {
                size += Serialize::serialize(&OutgoingStakingTransactionType::Unstake, writer)?;
                size += Serialize::serialize(proof, writer)?;
            }
        }
        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size = 0;
        match self {
            OutgoingStakingTransactionProof::DropValidator {
                validator_key,
                signature,
            } => {
                size += Serialize::serialized_size(&OutgoingStakingTransactionType::DropValidator);
                size += Serialize::serialized_size(validator_key);
                size += Serialize::serialized_size(signature);
            }
            OutgoingStakingTransactionProof::Unstake(proof) => {
                size += Serialize::serialized_size(&OutgoingStakingTransactionType::Unstake);
                size += Serialize::serialized_size(proof);
            }
        }
        size
    }
}

impl Deserialize for OutgoingStakingTransactionProof {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let ty: OutgoingStakingTransactionType = Deserialize::deserialize(reader)?;
        match ty {
            OutgoingStakingTransactionType::DropValidator => {
                Ok(OutgoingStakingTransactionProof::DropValidator {
                    validator_key: Deserialize::deserialize(reader)?,
                    signature: Deserialize::deserialize(reader)?,
                })
            }
            OutgoingStakingTransactionType::Unstake => Ok(
                OutgoingStakingTransactionProof::Unstake(Deserialize::deserialize(reader)?),
            ),
        }
    }
}

#[derive(Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum SelfStakingTransactionType {
    RetireStake = 0,
    ReactivateStake = 1,
}

#[derive(Clone, Debug)]
pub enum SelfStakingTransactionData {
    RetireStake(BlsPublicKey),
    ReactivateStake(BlsPublicKey),
}

impl SelfStakingTransactionData {
    pub fn parse(transaction: &Transaction) -> Result<Self, TransactionError> {
        full_parse(&transaction.data[..])
    }

    pub fn verify(&self, _transaction: &Transaction) -> Result<(), TransactionError> {
        Ok(())
    }
}

impl Serialize for SelfStakingTransactionData {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = 0;
        match self {
            SelfStakingTransactionData::RetireStake(validator_key) => {
                size += Serialize::serialize(&SelfStakingTransactionType::RetireStake, writer)?;
                size += Serialize::serialize(validator_key, writer)?;
            }
            SelfStakingTransactionData::ReactivateStake(validator_key) => {
                size += Serialize::serialize(&SelfStakingTransactionType::ReactivateStake, writer)?;
                size += Serialize::serialize(validator_key, writer)?;
            }
        }
        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size = 0;
        match self {
            SelfStakingTransactionData::RetireStake(validator_key) => {
                size += Serialize::serialized_size(&SelfStakingTransactionType::RetireStake);
                size += Serialize::serialized_size(validator_key);
            }
            SelfStakingTransactionData::ReactivateStake(validator_key) => {
                size += Serialize::serialized_size(&SelfStakingTransactionType::ReactivateStake);
                size += Serialize::serialized_size(validator_key);
            }
        }
        size
    }
}

impl Deserialize for SelfStakingTransactionData {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let ty: SelfStakingTransactionType = Deserialize::deserialize(reader)?;
        match ty {
            SelfStakingTransactionType::RetireStake => {
                let validator_key: BlsPublicKey = Deserialize::deserialize(reader)?;

                Ok(SelfStakingTransactionData::RetireStake(validator_key))
            }
            SelfStakingTransactionType::ReactivateStake => {
                let validator_key: BlsPublicKey = Deserialize::deserialize(reader)?;

                Ok(SelfStakingTransactionData::ReactivateStake(validator_key))
            }
        }
    }
}

pub fn full_parse<T: Deserialize>(mut data: &[u8]) -> Result<T, TransactionError> {
    let reader = &mut data;
    let data = Deserialize::deserialize(reader)?;

    // Ensure that transaction data has been fully read.
    if reader.read_u8().is_ok() {
        return Err(TransactionError::InvalidData);
    }

    Ok(data)
}

pub fn verify_transaction_signature(
    transaction: &Transaction,
    validator_key: &BlsPublicKey,
    signature: &BlsSignature,
) -> Result<(), TransactionError> {
    let key = validator_key
        .uncompress()
        .map_err(|_| TransactionError::InvalidProof)?;
    let sig = signature
        .uncompress()
        .map_err(|_| TransactionError::InvalidProof)?;

    // On incoming transactions, we need to reset the signature first.
    let tx = if transaction.recipient_type == AccountType::Staking {
        let mut tx_without_sig = transaction.clone();
        tx_without_sig.data = IncomingStakingTransactionData::set_validator_signature_on_data(
            &tx_without_sig.data,
            BlsSignature::default(),
        )?;
        tx_without_sig.serialize_content()
    } else {
        transaction.serialize_content()
    };
    if !key.verify(&tx, &sig) {
        warn!("Invalid signature");
        return Err(TransactionError::InvalidProof);
    }
    Ok(())
}

/// Important: Currently, the proof of knowledge of the secret key is a signature of the public key.
/// If an attacker A ever tricks a validator B into signing a message with content `pk_A - pk_B`,
/// where `pk_X` is X's BLS public key, A will be able to sign aggregate messages that are valid for
/// public keys `pk_B + (pk_A - pk_B) = pk_B`.
/// Alternatives would be to replace the proof of knowledge by a zero-knowledge proof.
pub fn verify_proof_of_knowledge(
    validator_key: &BlsPublicKey,
    proof_of_knowledge: &BlsSignature,
) -> Result<(), TransactionError> {
    if !validator_key
        .uncompress()
        .map_err(|_| TransactionError::InvalidData)?
        .verify(
            validator_key,
            &proof_of_knowledge
                .uncompress()
                .map_err(|_| TransactionError::InvalidData)?,
        )
    {
        return Err(TransactionError::InvalidData);
    }
    Ok(())
}
