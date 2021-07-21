use log::error;

use beserial::{Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};
use bls::{CompressedPublicKey as BlsPublicKey, CompressedSignature as BlsSignature};
use keys::Address;
use nimiq_hash::Blake2bHash;
use primitives::coin::Coin;
use primitives::policy;

use crate::SignatureProof;
use crate::{Transaction, TransactionError};

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
    CreateStaker = 5,
    Stake = 6,
    UpdateStaker = 7,
    RetireStaker = 8,
    ReactivateStaker = 9,
}

impl IncomingStakingTransactionType {
    pub fn is_signalling(&self) -> bool {
        matches!(
            self,
            IncomingStakingTransactionType::UpdateValidator
                | IncomingStakingTransactionType::RetireValidator
                | IncomingStakingTransactionType::ReactivateValidator
                | IncomingStakingTransactionType::UnparkValidator
                | IncomingStakingTransactionType::UpdateStaker
                | IncomingStakingTransactionType::RetireStaker
                | IncomingStakingTransactionType::ReactivateStaker
        )
    }
}

/// It is important to note that all `signature` fields contain the signature
/// over the complete transaction with the `signature` field set to `Default::default()`.
/// The field is populated only after computing the signature.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde-derive", derive(serde::Serialize, serde::Deserialize))]
pub enum IncomingStakingTransactionData {
    CreateValidator {
        warm_key: Address,
        validator_key: BlsPublicKey,
        reward_address: Address,
        signal_data: Option<Blake2bHash>,
        proof_of_knowledge: BlsSignature,
        signature: SignatureProof,
    },
    UpdateValidator {
        new_warm_key: Option<Address>,
        new_validator_key: Option<BlsPublicKey>,
        new_reward_address: Option<Address>,
        new_signal_data: Option<Option<Blake2bHash>>,
        new_proof_of_knowledge: Option<BlsSignature>,
        signature: SignatureProof,
    },
    RetireValidator {
        signature: SignatureProof,
    },
    ReactivateValidator {
        signature: SignatureProof,
    },
    UnparkValidator {
        signature: SignatureProof,
    },
    CreateStaker {
        delegation: Option<Address>,
        signature: SignatureProof,
    },
    Stake {
        staker_address: Address,
    },
    UpdateStaker {
        new_delegation: Option<Address>,
        signature: SignatureProof,
    },
    RetireStaker {
        value: Coin,
        signature: SignatureProof,
    },
    ReactivateStaker {
        value: Coin,
        signature: SignatureProof,
    },
}

impl IncomingStakingTransactionData {
    pub fn is_signalling(&self) -> bool {
        matches!(
            self,
            IncomingStakingTransactionData::UpdateValidator { .. }
                | IncomingStakingTransactionData::RetireValidator { .. }
                | IncomingStakingTransactionData::ReactivateValidator { .. }
                | IncomingStakingTransactionData::UnparkValidator { .. }
                | IncomingStakingTransactionData::UpdateStaker { .. }
                | IncomingStakingTransactionData::RetireStaker { .. }
                | IncomingStakingTransactionData::ReactivateStaker { .. }
        )
    }

    pub fn parse(transaction: &Transaction) -> Result<Self, TransactionError> {
        full_parse(&transaction.data[..])
    }

    pub fn verify(&self, transaction: &Transaction) -> Result<(), TransactionError> {
        match self {
            IncomingStakingTransactionData::CreateValidator {
                validator_key,
                proof_of_knowledge,
                signature,
                ..
            } => {
                // Validators must be created with exactly the validator deposit amount.
                if transaction.value != Coin::from_u64_unchecked(policy::VALIDATOR_DEPOSIT) {
                    warn!("Validator stake value different from VALIDATOR_DEPOSIT.");
                    return Err(TransactionError::InvalidForRecipient);
                }

                // Check proof of knowledge.
                verify_proof_of_knowledge(validator_key, proof_of_knowledge)?;

                // Check that the signature is correct.
                if !signature.verify(transaction.serialize_content().as_slice()) {
                    warn!("Invalid signature");
                    return Err(TransactionError::InvalidProof);
                }
            }
            IncomingStakingTransactionData::UpdateValidator {
                new_warm_key,
                new_validator_key,
                new_reward_address,
                new_signal_data,
                new_proof_of_knowledge,
                signature,
            } => {
                // Do not allow updates without any effect.
                if new_warm_key.is_none()
                    && new_validator_key.is_none()
                    && new_reward_address.is_none()
                    && new_signal_data.is_none()
                {
                    return Err(TransactionError::InvalidData);
                }

                // Check proof of knowledge, if necessary.
                if let (Some(new_validator_key), Some(new_proof_of_knowledge)) =
                    (new_validator_key, new_proof_of_knowledge)
                {
                    verify_proof_of_knowledge(new_validator_key, new_proof_of_knowledge)?;
                }

                // Check that the signature is correct.
                if !signature.verify(transaction.serialize_content().as_slice()) {
                    warn!("Invalid signature");
                    return Err(TransactionError::InvalidProof);
                }
            }
            IncomingStakingTransactionData::RetireValidator { signature } => {
                // Check that the signature is correct.
                if !signature.verify(transaction.serialize_content().as_slice()) {
                    warn!("Invalid signature");
                    return Err(TransactionError::InvalidProof);
                }
            }
            IncomingStakingTransactionData::ReactivateValidator { signature } => {
                // Check that the signature is correct.
                if !signature.verify(transaction.serialize_content().as_slice()) {
                    warn!("Invalid signature");
                    return Err(TransactionError::InvalidProof);
                }
            }
            IncomingStakingTransactionData::UnparkValidator { signature } => {
                // Check that the signature is correct.
                if !signature.verify(transaction.serialize_content().as_slice()) {
                    warn!("Invalid signature");
                    return Err(TransactionError::InvalidProof);
                }
            }
            IncomingStakingTransactionData::CreateStaker { .. } => {
                // Check that stake is bigger than zero.
                if transaction.value.is_zero() {
                    warn!("Can't create a staker with zero balance.");
                    return Err(TransactionError::InvalidForRecipient);
                }
            }
            IncomingStakingTransactionData::Stake { .. } => {
                // No checks needed.
            }
            IncomingStakingTransactionData::UpdateStaker { signature, .. } => {
                // Check that the signature is correct.
                if !signature.verify(transaction.serialize_content().as_slice()) {
                    warn!("Invalid signature");
                    return Err(TransactionError::InvalidProof);
                }
            }
            IncomingStakingTransactionData::RetireStaker { value, signature } => {
                // Check that the value to be retired is bigger than zero.
                if value.is_zero() {
                    warn!("Can't retire zero stake.");
                    return Err(TransactionError::InvalidForRecipient);
                }

                // Check that the signature is correct.
                if !signature.verify(transaction.serialize_content().as_slice()) {
                    warn!("Invalid signature");
                    return Err(TransactionError::InvalidProof);
                }
            }
            IncomingStakingTransactionData::ReactivateStaker { value, signature } => {
                // Check that the value to be reactivated is bigger than zero.
                if value.is_zero() {
                    warn!("Can't reactivate zero stake.");
                    return Err(TransactionError::InvalidForRecipient);
                }

                // Check that the signature is correct.
                if !signature.verify(transaction.serialize_content().as_slice()) {
                    warn!("Invalid signature");
                    return Err(TransactionError::InvalidProof);
                }
            }
        }

        Ok(())
    }

    pub fn set_signature(&mut self, signature_proof: SignatureProof) {
        match self {
            IncomingStakingTransactionData::CreateValidator { signature, .. } => {
                *signature = signature_proof;
            }
            IncomingStakingTransactionData::UpdateValidator { signature, .. } => {
                *signature = signature_proof;
            }
            IncomingStakingTransactionData::RetireValidator { signature, .. } => {
                *signature = signature_proof;
            }
            IncomingStakingTransactionData::ReactivateValidator { signature, .. } => {
                *signature = signature_proof;
            }
            IncomingStakingTransactionData::UnparkValidator { signature, .. } => {
                *signature = signature_proof;
            }
            IncomingStakingTransactionData::UpdateStaker { signature, .. } => {
                *signature = signature_proof;
            }
            IncomingStakingTransactionData::RetireStaker { signature, .. } => {
                *signature = signature_proof;
            }
            IncomingStakingTransactionData::ReactivateStaker { signature, .. } => {
                *signature = signature_proof;
            }
            _ => {}
        }
    }

    pub fn set_signature_on_data(
        data: &[u8],
        signature_proof: SignatureProof,
    ) -> Result<Vec<u8>, SerializingError> {
        let mut data: IncomingStakingTransactionData = Deserialize::deserialize_from_vec(data)?;
        data.set_signature(signature_proof);
        Ok(data.serialize_to_vec())
    }
}

impl Serialize for IncomingStakingTransactionData {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = 0;
        match self {
            IncomingStakingTransactionData::CreateValidator {
                warm_key,
                validator_key,
                reward_address,
                signal_data,
                proof_of_knowledge,
                signature,
            } => {
                size +=
                    Serialize::serialize(&IncomingStakingTransactionType::CreateValidator, writer)?;
                size += Serialize::serialize(warm_key, writer)?;
                size += Serialize::serialize(validator_key, writer)?;
                size += Serialize::serialize(reward_address, writer)?;
                size += Serialize::serialize(signal_data, writer)?;
                size += Serialize::serialize(proof_of_knowledge, writer)?;
                size += Serialize::serialize(signature, writer)?;
            }
            IncomingStakingTransactionData::UpdateValidator {
                new_warm_key,
                new_validator_key,
                new_reward_address,
                new_signal_data,
                new_proof_of_knowledge,
                signature,
            } => {
                size +=
                    Serialize::serialize(&IncomingStakingTransactionType::UpdateValidator, writer)?;
                size += Serialize::serialize(&new_warm_key, writer)?;
                size += Serialize::serialize(&new_validator_key, writer)?;
                size += Serialize::serialize(&new_reward_address, writer)?;
                size += Serialize::serialize(&new_signal_data, writer)?;
                size += Serialize::serialize(&new_proof_of_knowledge, writer)?;
                size += Serialize::serialize(signature, writer)?;
            }
            IncomingStakingTransactionData::RetireValidator { signature } => {
                size +=
                    Serialize::serialize(&IncomingStakingTransactionType::RetireValidator, writer)?;
                size += Serialize::serialize(signature, writer)?;
            }
            IncomingStakingTransactionData::ReactivateValidator { signature } => {
                size += Serialize::serialize(
                    &IncomingStakingTransactionType::ReactivateValidator,
                    writer,
                )?;
                size += Serialize::serialize(signature, writer)?;
            }
            IncomingStakingTransactionData::UnparkValidator { signature } => {
                size +=
                    Serialize::serialize(&IncomingStakingTransactionType::UnparkValidator, writer)?;
                size += Serialize::serialize(signature, writer)?;
            }
            IncomingStakingTransactionData::CreateStaker {
                delegation,
                signature,
            } => {
                size += Serialize::serialize(&IncomingStakingTransactionType::Stake, writer)?;
                size += Serialize::serialize(delegation, writer)?;
                size += Serialize::serialize(signature, writer)?;
            }
            IncomingStakingTransactionData::Stake { staker_address } => {
                size += Serialize::serialize(&IncomingStakingTransactionType::Stake, writer)?;
                size += Serialize::serialize(staker_address, writer)?;
            }
            IncomingStakingTransactionData::UpdateStaker {
                new_delegation,
                signature,
            } => {
                size += Serialize::serialize(&IncomingStakingTransactionType::Stake, writer)?;
                size += Serialize::serialize(new_delegation, writer)?;
                size += Serialize::serialize(signature, writer)?;
            }
            IncomingStakingTransactionData::RetireStaker { value, signature } => {
                size += Serialize::serialize(&IncomingStakingTransactionType::Stake, writer)?;
                size += Serialize::serialize(value, writer)?;
                size += Serialize::serialize(signature, writer)?;
            }
            IncomingStakingTransactionData::ReactivateStaker { value, signature } => {
                size += Serialize::serialize(&IncomingStakingTransactionType::Stake, writer)?;
                size += Serialize::serialize(value, writer)?;
                size += Serialize::serialize(signature, writer)?;
            }
        }
        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size = 0;
        match self {
            IncomingStakingTransactionData::CreateValidator {
                warm_key,
                validator_key,
                reward_address,
                signal_data,
                proof_of_knowledge,
                signature,
            } => {
                size +=
                    Serialize::serialized_size(&IncomingStakingTransactionType::CreateValidator);
                size += Serialize::serialized_size(warm_key);
                size += Serialize::serialized_size(validator_key);
                size += Serialize::serialized_size(reward_address);
                size += Serialize::serialized_size(signal_data);
                size += Serialize::serialized_size(proof_of_knowledge);
                size += Serialize::serialized_size(signature);
            }
            IncomingStakingTransactionData::UpdateValidator {
                new_warm_key,
                new_validator_key,
                new_reward_address,
                new_signal_data,
                new_proof_of_knowledge,
                signature,
            } => {
                size +=
                    Serialize::serialized_size(&IncomingStakingTransactionType::UpdateValidator);
                size += Serialize::serialized_size(new_warm_key);
                size += Serialize::serialized_size(new_validator_key);
                size += Serialize::serialized_size(new_reward_address);
                size += Serialize::serialized_size(new_signal_data);
                size += Serialize::serialized_size(new_proof_of_knowledge);
                size += Serialize::serialized_size(signature);
            }
            IncomingStakingTransactionData::RetireValidator { signature } => {
                size +=
                    Serialize::serialized_size(&IncomingStakingTransactionType::RetireValidator);
                size += Serialize::serialized_size(signature);
            }
            IncomingStakingTransactionData::ReactivateValidator { signature } => {
                size += Serialize::serialized_size(
                    &IncomingStakingTransactionType::ReactivateValidator,
                );
                size += Serialize::serialized_size(signature);
            }
            IncomingStakingTransactionData::UnparkValidator { signature } => {
                size +=
                    Serialize::serialized_size(&IncomingStakingTransactionType::UnparkValidator);
                size += Serialize::serialized_size(signature);
            }
            IncomingStakingTransactionData::CreateStaker {
                delegation,
                signature,
            } => {
                size += Serialize::serialized_size(&IncomingStakingTransactionType::Stake);
                size += Serialize::serialized_size(delegation);
                size += Serialize::serialized_size(signature);
            }
            IncomingStakingTransactionData::Stake { staker_address } => {
                size += Serialize::serialized_size(&IncomingStakingTransactionType::Stake);
                size += Serialize::serialized_size(staker_address);
            }
            IncomingStakingTransactionData::UpdateStaker {
                new_delegation,
                signature,
            } => {
                size += Serialize::serialized_size(&IncomingStakingTransactionType::Stake);
                size += Serialize::serialized_size(new_delegation);
                size += Serialize::serialized_size(signature);
            }
            IncomingStakingTransactionData::RetireStaker { value, signature } => {
                size += Serialize::serialized_size(&IncomingStakingTransactionType::Stake);
                size += Serialize::serialized_size(value);
                size += Serialize::serialized_size(signature);
            }
            IncomingStakingTransactionData::ReactivateStaker { value, signature } => {
                size += Serialize::serialized_size(&IncomingStakingTransactionType::Stake);
                size += Serialize::serialized_size(value);
                size += Serialize::serialized_size(signature);
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
                    warm_key: Deserialize::deserialize(reader)?,
                    validator_key: Deserialize::deserialize(reader)?,
                    reward_address: Deserialize::deserialize(reader)?,
                    signal_data: Deserialize::deserialize(reader)?,
                    proof_of_knowledge: Deserialize::deserialize(reader)?,
                    signature: Deserialize::deserialize(reader)?,
                })
            }
            IncomingStakingTransactionType::UpdateValidator => {
                Ok(IncomingStakingTransactionData::UpdateValidator {
                    new_warm_key: Deserialize::deserialize(reader)?,
                    new_validator_key: Deserialize::deserialize(reader)?,
                    new_reward_address: Deserialize::deserialize(reader)?,
                    new_signal_data: Deserialize::deserialize(reader)?,
                    new_proof_of_knowledge: Deserialize::deserialize(reader)?,
                    signature: Deserialize::deserialize(reader)?,
                })
            }
            IncomingStakingTransactionType::RetireValidator => {
                Ok(IncomingStakingTransactionData::RetireValidator {
                    signature: Deserialize::deserialize(reader)?,
                })
            }
            IncomingStakingTransactionType::ReactivateValidator => {
                Ok(IncomingStakingTransactionData::ReactivateValidator {
                    signature: Deserialize::deserialize(reader)?,
                })
            }
            IncomingStakingTransactionType::UnparkValidator => {
                Ok(IncomingStakingTransactionData::UnparkValidator {
                    signature: Deserialize::deserialize(reader)?,
                })
            }
            IncomingStakingTransactionType::CreateStaker => {
                Ok(IncomingStakingTransactionData::CreateStaker {
                    delegation: Deserialize::deserialize(reader)?,
                    signature: Deserialize::deserialize(reader)?,
                })
            }
            IncomingStakingTransactionType::Stake => Ok(IncomingStakingTransactionData::Stake {
                staker_address: Deserialize::deserialize(reader)?,
            }),
            IncomingStakingTransactionType::UpdateStaker => {
                Ok(IncomingStakingTransactionData::UpdateStaker {
                    new_delegation: Deserialize::deserialize(reader)?,
                    signature: Deserialize::deserialize(reader)?,
                })
            }
            IncomingStakingTransactionType::RetireStaker => {
                Ok(IncomingStakingTransactionData::RetireStaker {
                    value: Deserialize::deserialize(reader)?,
                    signature: Deserialize::deserialize(reader)?,
                })
            }
            IncomingStakingTransactionType::ReactivateStaker => {
                Ok(IncomingStakingTransactionData::ReactivateStaker {
                    value: Deserialize::deserialize(reader)?,
                    signature: Deserialize::deserialize(reader)?,
                })
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum OutgoingStakingTransactionType {
    DropValidator = 0,
    Unstake = 1,
    DeductFees = 2,
}

#[derive(Clone, Debug)]
pub enum OutgoingStakingTransactionProof {
    DropValidator {
        signature: SignatureProof,
    },
    Unstake {
        signature: SignatureProof,
    },
    DeductFees {
        from_active_balance: bool,
        signature: SignatureProof,
    },
}

impl OutgoingStakingTransactionProof {
    pub fn parse(transaction: &Transaction) -> Result<Self, TransactionError> {
        full_parse(&transaction.proof[..])
    }

    pub fn verify(&self, transaction: &Transaction) -> Result<(), TransactionError> {
        match self {
            OutgoingStakingTransactionProof::DropValidator { signature } => {
                // When dropping a validator you get exactly the validator deposit back.
                if transaction.total_value()? != Coin::from_u64_unchecked(policy::VALIDATOR_DEPOSIT)
                {
                    warn!("Wrong value when dropping a validator. It must be VALIDATOR_DEPOSIT.");
                    return Err(TransactionError::InvalidForSender);
                }

                // Check that the signature is correct.
                if !signature.verify(transaction.serialize_content().as_slice()) {
                    warn!("Invalid signature");
                    return Err(TransactionError::InvalidProof);
                }

                Ok(())
            }
            OutgoingStakingTransactionProof::Unstake { signature } => {
                // Check that the signature is correct.
                if !signature.verify(transaction.serialize_content().as_slice()) {
                    warn!("Invalid signature");
                    return Err(TransactionError::InvalidProof);
                }

                Ok(())
            }
            OutgoingStakingTransactionProof::DeductFees { signature, .. } => {
                // We need to check that this is only used to pay fees and not to transfer value.
                if !transaction.value.is_zero() {
                    warn!("Trying to transfer value when calling the DeductFees interaction. You can't do this!");
                    return Err(TransactionError::InvalidForSender);
                }

                // Check that the signature is correct.
                if !signature.verify(transaction.serialize_content().as_slice()) {
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
            OutgoingStakingTransactionProof::DropValidator { signature } => {
                size +=
                    Serialize::serialize(&OutgoingStakingTransactionType::DropValidator, writer)?;
                size += Serialize::serialize(signature, writer)?;
            }
            OutgoingStakingTransactionProof::Unstake { signature } => {
                size += Serialize::serialize(&OutgoingStakingTransactionType::Unstake, writer)?;
                size += Serialize::serialize(signature, writer)?;
            }
            OutgoingStakingTransactionProof::DeductFees {
                from_active_balance,
                signature,
            } => {
                size += Serialize::serialize(&OutgoingStakingTransactionType::DeductFees, writer)?;
                size += Serialize::serialize(from_active_balance, writer)?;
                size += Serialize::serialize(signature, writer)?;
            }
        }
        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size = 0;
        match self {
            OutgoingStakingTransactionProof::DropValidator { signature } => {
                size += Serialize::serialized_size(&OutgoingStakingTransactionType::DropValidator);
                size += Serialize::serialized_size(signature);
            }
            OutgoingStakingTransactionProof::Unstake { signature } => {
                size += Serialize::serialized_size(&OutgoingStakingTransactionType::Unstake);
                size += Serialize::serialized_size(signature);
            }
            OutgoingStakingTransactionProof::DeductFees {
                from_active_balance,
                signature,
            } => {
                size += Serialize::serialized_size(&OutgoingStakingTransactionType::DeductFees);
                size += Serialize::serialized_size(from_active_balance);
                size += Serialize::serialized_size(signature);
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
                    signature: Deserialize::deserialize(reader)?,
                })
            }
            OutgoingStakingTransactionType::Unstake => {
                Ok(OutgoingStakingTransactionProof::Unstake {
                    signature: Deserialize::deserialize(reader)?,
                })
            }
            OutgoingStakingTransactionType::DeductFees => {
                Ok(OutgoingStakingTransactionProof::DeductFees {
                    from_active_balance: Deserialize::deserialize(reader)?,
                    signature: Deserialize::deserialize(reader)?,
                })
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
        error!("Verification of the proof of knowledge for a BLS key failed!");
        return Err(TransactionError::InvalidData);
    }

    Ok(())
}
