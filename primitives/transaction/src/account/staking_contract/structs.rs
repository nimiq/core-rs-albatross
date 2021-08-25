use log::error;

use beserial::{Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};
use bls::{CompressedPublicKey as BlsPublicKey, CompressedSignature as BlsSignature};
use keys::Address;
use nimiq_hash::Blake2bHash;
use primitives::coin::Coin;
use primitives::policy;

use crate::SignatureProof;
use crate::{Transaction, TransactionError};

/// We need to distinguish two types of transactions:
/// 1. Incoming transactions, which include:
///     - Validator
///         * Create
///         * Update
///         * Retire
///         * Reactivate
///         * Unpark
///     - Staker
///         * Create
///         * Stake
///         * Update
///         * Retire
///         * Reactivate
///     The type of transaction, parameters and proof are given in the data field of the transaction.
/// 2. Outgoing transactions, which include:
///     - Validator
///         * Drop
///     - Staker
///         * Unstake
///         * Deduct fees
///     The type of transaction, parameters and proof are given in the proof field of the transaction.
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
        // This proof is signed with the validator cold key, which will become the validator address.
        #[cfg_attr(feature = "serde-derive", serde(skip))]
        proof: SignatureProof,
    },
    UpdateValidator {
        new_warm_key: Option<Address>,
        new_validator_key: Option<BlsPublicKey>,
        new_reward_address: Option<Address>,
        new_signal_data: Option<Option<Blake2bHash>>,
        new_proof_of_knowledge: Option<BlsSignature>,
        // This proof is signed with the validator cold key.
        #[cfg_attr(feature = "serde-derive", serde(skip))]
        proof: SignatureProof,
    },
    RetireValidator {
        validator_address: Address,
        // This proof is signed with the validator warm key.
        #[cfg_attr(feature = "serde-derive", serde(skip))]
        proof: SignatureProof,
    },
    ReactivateValidator {
        validator_address: Address,
        // This proof is signed with the validator warm key.
        #[cfg_attr(feature = "serde-derive", serde(skip))]
        proof: SignatureProof,
    },
    UnparkValidator {
        validator_address: Address,
        // This proof is signed with the validator warm key.
        #[cfg_attr(feature = "serde-derive", serde(skip))]
        proof: SignatureProof,
    },
    CreateStaker {
        delegation: Option<Address>,
        #[cfg_attr(feature = "serde-derive", serde(skip))]
        proof: SignatureProof,
    },
    Stake {
        staker_address: Address,
    },
    UpdateStaker {
        new_delegation: Option<Address>,
        #[cfg_attr(feature = "serde-derive", serde(skip))]
        proof: SignatureProof,
    },
    RetireStaker {
        value: Coin,
        #[cfg_attr(feature = "serde-derive", serde(skip))]
        proof: SignatureProof,
    },
    ReactivateStaker {
        value: Coin,
        #[cfg_attr(feature = "serde-derive", serde(skip))]
        proof: SignatureProof,
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
                proof,
                ..
            } => {
                // Validators must be created with exactly the validator deposit amount.
                if transaction.value != Coin::from_u64_unchecked(policy::VALIDATOR_DEPOSIT) {
                    error!("Validator stake value different from VALIDATOR_DEPOSIT. The offending transaction is the following:\n{:?}", transaction);
                    return Err(TransactionError::InvalidValue);
                }

                // Check proof of knowledge.
                verify_proof_of_knowledge(validator_key, proof_of_knowledge)?;

                // Check that the signature is correct.
                verify_transaction_signature(transaction, proof, true)?
            }
            IncomingStakingTransactionData::UpdateValidator {
                new_warm_key,
                new_validator_key,
                new_reward_address,
                new_signal_data,
                new_proof_of_knowledge,
                proof,
            } => {
                // Do not allow updates without any effect.
                if new_warm_key.is_none()
                    && new_validator_key.is_none()
                    && new_reward_address.is_none()
                    && new_signal_data.is_none()
                {
                    error!("Signalling update transactions must actually update something. The offending transaction is the following:\n{:?}", transaction);
                    return Err(TransactionError::InvalidData);
                }

                // Check proof of knowledge, if necessary.
                if let (Some(new_validator_key), Some(new_proof_of_knowledge)) =
                    (new_validator_key, new_proof_of_knowledge)
                {
                    verify_proof_of_knowledge(new_validator_key, new_proof_of_knowledge)?;
                }

                // Check that the signature is correct.
                verify_transaction_signature(transaction, proof, true)?
            }
            IncomingStakingTransactionData::RetireValidator { proof, .. } => {
                // Check that the signature is correct.
                verify_transaction_signature(transaction, proof, true)?
            }
            IncomingStakingTransactionData::ReactivateValidator { proof, .. } => {
                // Check that the signature is correct.
                verify_transaction_signature(transaction, proof, true)?
            }
            IncomingStakingTransactionData::UnparkValidator { proof, .. } => {
                // Check that the signature is correct.
                verify_transaction_signature(transaction, proof, true)?
            }
            IncomingStakingTransactionData::CreateStaker { proof, .. } => {
                // Check that stake is bigger than zero.
                if transaction.value.is_zero() {
                    warn!("Can't create a staker with zero balance. The offending transaction is the following:\n{:?}", transaction);
                    return Err(TransactionError::ZeroValue);
                }

                // Check that the signature is correct.
                verify_transaction_signature(transaction, proof, true)?
            }
            IncomingStakingTransactionData::Stake { .. } => {
                // No checks needed.
            }
            IncomingStakingTransactionData::UpdateStaker { proof, .. } => {
                // Check that the signature is correct.
                verify_transaction_signature(transaction, proof, true)?
            }
            IncomingStakingTransactionData::RetireStaker { value, proof } => {
                // Check that the value to be retired is bigger than zero.
                if value.is_zero() {
                    warn!("Can't retire zero stake. The offending transaction is the following:\n{:?}", transaction);
                    return Err(TransactionError::InvalidData);
                }

                // Check that the signature is correct.
                verify_transaction_signature(transaction, proof, true)?
            }
            IncomingStakingTransactionData::ReactivateStaker { value, proof } => {
                // Check that the value to be reactivated is bigger than zero.
                if value.is_zero() {
                    warn!("Can't reactivate zero stake. The offending transaction is the following:\n{:?}", transaction);
                    return Err(TransactionError::InvalidData);
                }

                // Check that the signature is correct.
                verify_transaction_signature(transaction, proof, true)?
            }
        }

        Ok(())
    }

    pub fn set_signature(&mut self, signature_proof: SignatureProof) {
        match self {
            IncomingStakingTransactionData::CreateValidator { proof, .. } => {
                *proof = signature_proof;
            }
            IncomingStakingTransactionData::UpdateValidator { proof, .. } => {
                *proof = signature_proof;
            }
            IncomingStakingTransactionData::RetireValidator { proof, .. } => {
                *proof = signature_proof;
            }
            IncomingStakingTransactionData::ReactivateValidator { proof, .. } => {
                *proof = signature_proof;
            }
            IncomingStakingTransactionData::UnparkValidator { proof, .. } => {
                *proof = signature_proof;
            }
            IncomingStakingTransactionData::CreateStaker { proof, .. } => {
                *proof = signature_proof;
            }
            IncomingStakingTransactionData::UpdateStaker { proof, .. } => {
                *proof = signature_proof;
            }
            IncomingStakingTransactionData::RetireStaker { proof, .. } => {
                *proof = signature_proof;
            }
            IncomingStakingTransactionData::ReactivateStaker { proof, .. } => {
                *proof = signature_proof;
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
                proof,
            } => {
                size +=
                    Serialize::serialize(&IncomingStakingTransactionType::CreateValidator, writer)?;
                size += Serialize::serialize(warm_key, writer)?;
                size += Serialize::serialize(validator_key, writer)?;
                size += Serialize::serialize(reward_address, writer)?;
                size += Serialize::serialize(signal_data, writer)?;
                size += Serialize::serialize(proof_of_knowledge, writer)?;
                size += Serialize::serialize(proof, writer)?;
            }
            IncomingStakingTransactionData::UpdateValidator {
                new_warm_key,
                new_validator_key,
                new_reward_address,
                new_signal_data,
                new_proof_of_knowledge,
                proof,
            } => {
                size +=
                    Serialize::serialize(&IncomingStakingTransactionType::UpdateValidator, writer)?;
                size += Serialize::serialize(&new_warm_key, writer)?;
                size += Serialize::serialize(&new_validator_key, writer)?;
                size += Serialize::serialize(&new_reward_address, writer)?;
                size += Serialize::serialize(&new_signal_data, writer)?;
                size += Serialize::serialize(&new_proof_of_knowledge, writer)?;
                size += Serialize::serialize(proof, writer)?;
            }
            IncomingStakingTransactionData::RetireValidator {
                validator_address,
                proof,
            } => {
                size +=
                    Serialize::serialize(&IncomingStakingTransactionType::RetireValidator, writer)?;
                size += Serialize::serialize(validator_address, writer)?;
                size += Serialize::serialize(proof, writer)?;
            }
            IncomingStakingTransactionData::ReactivateValidator {
                validator_address,
                proof,
            } => {
                size += Serialize::serialize(
                    &IncomingStakingTransactionType::ReactivateValidator,
                    writer,
                )?;
                size += Serialize::serialize(validator_address, writer)?;
                size += Serialize::serialize(proof, writer)?;
            }
            IncomingStakingTransactionData::UnparkValidator {
                validator_address,
                proof,
            } => {
                size +=
                    Serialize::serialize(&IncomingStakingTransactionType::UnparkValidator, writer)?;
                size += Serialize::serialize(validator_address, writer)?;
                size += Serialize::serialize(proof, writer)?;
            }
            IncomingStakingTransactionData::CreateStaker { delegation, proof } => {
                size +=
                    Serialize::serialize(&IncomingStakingTransactionType::CreateStaker, writer)?;
                size += Serialize::serialize(delegation, writer)?;
                size += Serialize::serialize(proof, writer)?;
            }
            IncomingStakingTransactionData::Stake { staker_address } => {
                size += Serialize::serialize(&IncomingStakingTransactionType::Stake, writer)?;
                size += Serialize::serialize(staker_address, writer)?;
            }
            IncomingStakingTransactionData::UpdateStaker {
                new_delegation,
                proof,
            } => {
                size +=
                    Serialize::serialize(&IncomingStakingTransactionType::UpdateStaker, writer)?;
                size += Serialize::serialize(new_delegation, writer)?;
                size += Serialize::serialize(proof, writer)?;
            }
            IncomingStakingTransactionData::RetireStaker { value, proof } => {
                size +=
                    Serialize::serialize(&IncomingStakingTransactionType::RetireStaker, writer)?;
                size += Serialize::serialize(value, writer)?;
                size += Serialize::serialize(proof, writer)?;
            }
            IncomingStakingTransactionData::ReactivateStaker { value, proof } => {
                size += Serialize::serialize(
                    &IncomingStakingTransactionType::ReactivateStaker,
                    writer,
                )?;
                size += Serialize::serialize(value, writer)?;
                size += Serialize::serialize(proof, writer)?;
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
                proof,
            } => {
                size +=
                    Serialize::serialized_size(&IncomingStakingTransactionType::CreateValidator);
                size += Serialize::serialized_size(warm_key);
                size += Serialize::serialized_size(validator_key);
                size += Serialize::serialized_size(reward_address);
                size += Serialize::serialized_size(signal_data);
                size += Serialize::serialized_size(proof_of_knowledge);
                size += Serialize::serialized_size(proof);
            }
            IncomingStakingTransactionData::UpdateValidator {
                new_warm_key,
                new_validator_key,
                new_reward_address,
                new_signal_data,
                new_proof_of_knowledge,
                proof,
            } => {
                size +=
                    Serialize::serialized_size(&IncomingStakingTransactionType::UpdateValidator);
                size += Serialize::serialized_size(new_warm_key);
                size += Serialize::serialized_size(new_validator_key);
                size += Serialize::serialized_size(new_reward_address);
                size += Serialize::serialized_size(new_signal_data);
                size += Serialize::serialized_size(new_proof_of_knowledge);
                size += Serialize::serialized_size(proof);
            }
            IncomingStakingTransactionData::RetireValidator {
                validator_address,
                proof,
            } => {
                size +=
                    Serialize::serialized_size(&IncomingStakingTransactionType::RetireValidator);
                size += Serialize::serialized_size(validator_address);
                size += Serialize::serialized_size(proof);
            }
            IncomingStakingTransactionData::ReactivateValidator {
                validator_address,
                proof,
            } => {
                size += Serialize::serialized_size(
                    &IncomingStakingTransactionType::ReactivateValidator,
                );
                size += Serialize::serialized_size(validator_address);
                size += Serialize::serialized_size(proof);
            }
            IncomingStakingTransactionData::UnparkValidator {
                validator_address,
                proof,
            } => {
                size +=
                    Serialize::serialized_size(&IncomingStakingTransactionType::UnparkValidator);
                size += Serialize::serialized_size(validator_address);
                size += Serialize::serialized_size(proof);
            }
            IncomingStakingTransactionData::CreateStaker { delegation, proof } => {
                size += Serialize::serialized_size(&IncomingStakingTransactionType::CreateStaker);
                size += Serialize::serialized_size(delegation);
                size += Serialize::serialized_size(proof);
            }
            IncomingStakingTransactionData::Stake { staker_address } => {
                size += Serialize::serialized_size(&IncomingStakingTransactionType::Stake);
                size += Serialize::serialized_size(staker_address);
            }
            IncomingStakingTransactionData::UpdateStaker {
                new_delegation,
                proof,
            } => {
                size += Serialize::serialized_size(&IncomingStakingTransactionType::UpdateStaker);
                size += Serialize::serialized_size(new_delegation);
                size += Serialize::serialized_size(proof);
            }
            IncomingStakingTransactionData::RetireStaker { value, proof } => {
                size += Serialize::serialized_size(&IncomingStakingTransactionType::RetireStaker);
                size += Serialize::serialized_size(value);
                size += Serialize::serialized_size(proof);
            }
            IncomingStakingTransactionData::ReactivateStaker { value, proof } => {
                size +=
                    Serialize::serialized_size(&IncomingStakingTransactionType::ReactivateStaker);
                size += Serialize::serialized_size(value);
                size += Serialize::serialized_size(proof);
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
                    proof: Deserialize::deserialize(reader)?,
                })
            }
            IncomingStakingTransactionType::UpdateValidator => {
                Ok(IncomingStakingTransactionData::UpdateValidator {
                    new_warm_key: Deserialize::deserialize(reader)?,
                    new_validator_key: Deserialize::deserialize(reader)?,
                    new_reward_address: Deserialize::deserialize(reader)?,
                    new_signal_data: Deserialize::deserialize(reader)?,
                    new_proof_of_knowledge: Deserialize::deserialize(reader)?,
                    proof: Deserialize::deserialize(reader)?,
                })
            }
            IncomingStakingTransactionType::RetireValidator => {
                Ok(IncomingStakingTransactionData::RetireValidator {
                    validator_address: Deserialize::deserialize(reader)?,
                    proof: Deserialize::deserialize(reader)?,
                })
            }
            IncomingStakingTransactionType::ReactivateValidator => {
                Ok(IncomingStakingTransactionData::ReactivateValidator {
                    validator_address: Deserialize::deserialize(reader)?,
                    proof: Deserialize::deserialize(reader)?,
                })
            }
            IncomingStakingTransactionType::UnparkValidator => {
                Ok(IncomingStakingTransactionData::UnparkValidator {
                    validator_address: Deserialize::deserialize(reader)?,
                    proof: Deserialize::deserialize(reader)?,
                })
            }
            IncomingStakingTransactionType::CreateStaker => {
                Ok(IncomingStakingTransactionData::CreateStaker {
                    delegation: Deserialize::deserialize(reader)?,
                    proof: Deserialize::deserialize(reader)?,
                })
            }
            IncomingStakingTransactionType::Stake => Ok(IncomingStakingTransactionData::Stake {
                staker_address: Deserialize::deserialize(reader)?,
            }),
            IncomingStakingTransactionType::UpdateStaker => {
                Ok(IncomingStakingTransactionData::UpdateStaker {
                    new_delegation: Deserialize::deserialize(reader)?,
                    proof: Deserialize::deserialize(reader)?,
                })
            }
            IncomingStakingTransactionType::RetireStaker => {
                Ok(IncomingStakingTransactionData::RetireStaker {
                    value: Deserialize::deserialize(reader)?,
                    proof: Deserialize::deserialize(reader)?,
                })
            }
            IncomingStakingTransactionType::ReactivateStaker => {
                Ok(IncomingStakingTransactionData::ReactivateStaker {
                    value: Deserialize::deserialize(reader)?,
                    proof: Deserialize::deserialize(reader)?,
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
        // This proof is signed with the validator cold key.
        proof: SignatureProof,
    },
    Unstake {
        proof: SignatureProof,
    },
    DeductFees {
        from_active_balance: bool,
        proof: SignatureProof,
    },
}

impl OutgoingStakingTransactionProof {
    pub fn parse(transaction: &Transaction) -> Result<Self, TransactionError> {
        full_parse(&transaction.proof[..])
    }

    pub fn verify(&self, transaction: &Transaction) -> Result<(), TransactionError> {
        match self {
            OutgoingStakingTransactionProof::DropValidator { proof } => {
                // When dropping a validator you get exactly the validator deposit back.
                if transaction.total_value()? != Coin::from_u64_unchecked(policy::VALIDATOR_DEPOSIT)
                {
                    error!("Wrong value when dropping a validator. It must be VALIDATOR_DEPOSIT. The offending transaction is the following:\n{:?}", transaction);
                    return Err(TransactionError::InvalidValue);
                }

                // Check that the signature is correct.
                verify_transaction_signature(transaction, proof, false)?
            }
            OutgoingStakingTransactionProof::Unstake { proof } => {
                // Check that the signature is correct.
                verify_transaction_signature(transaction, proof, false)?
            }
            OutgoingStakingTransactionProof::DeductFees { proof, .. } => {
                // We need to check that this is only used to pay fees and not to transfer value.
                if !transaction.value.is_zero() {
                    error!("Not allowed to transfer value when calling the DeductFees interaction. The offending transaction is the following:\n{:?}", transaction);
                    return Err(TransactionError::InvalidValue);
                }

                // Check that the signature is correct.
                verify_transaction_signature(transaction, proof, false)?
            }
        }

        Ok(())
    }
}

impl Serialize for OutgoingStakingTransactionProof {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = 0;
        match self {
            OutgoingStakingTransactionProof::DropValidator { proof: signature } => {
                size +=
                    Serialize::serialize(&OutgoingStakingTransactionType::DropValidator, writer)?;
                size += Serialize::serialize(signature, writer)?;
            }
            OutgoingStakingTransactionProof::Unstake { proof: signature } => {
                size += Serialize::serialize(&OutgoingStakingTransactionType::Unstake, writer)?;
                size += Serialize::serialize(signature, writer)?;
            }
            OutgoingStakingTransactionProof::DeductFees {
                from_active_balance,
                proof: signature,
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
            OutgoingStakingTransactionProof::DropValidator { proof: signature } => {
                size += Serialize::serialized_size(&OutgoingStakingTransactionType::DropValidator);
                size += Serialize::serialized_size(signature);
            }
            OutgoingStakingTransactionProof::Unstake { proof: signature } => {
                size += Serialize::serialized_size(&OutgoingStakingTransactionType::Unstake);
                size += Serialize::serialized_size(signature);
            }
            OutgoingStakingTransactionProof::DeductFees {
                from_active_balance,
                proof: signature,
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
                    proof: Deserialize::deserialize(reader)?,
                })
            }
            OutgoingStakingTransactionType::Unstake => {
                Ok(OutgoingStakingTransactionProof::Unstake {
                    proof: Deserialize::deserialize(reader)?,
                })
            }
            OutgoingStakingTransactionType::DeductFees => {
                Ok(OutgoingStakingTransactionProof::DeductFees {
                    from_active_balance: Deserialize::deserialize(reader)?,
                    proof: Deserialize::deserialize(reader)?,
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

pub fn verify_transaction_signature(
    transaction: &Transaction,
    sig_proof: &SignatureProof,
    incoming: bool,
) -> Result<(), TransactionError> {
    // If we are verifying the signature on an incoming transaction, then we need to reset the
    // signature field first.
    let tx = if incoming {
        let mut tx_without_sig = transaction.clone();

        tx_without_sig.data = IncomingStakingTransactionData::set_signature_on_data(
            &tx_without_sig.data,
            SignatureProof::default(),
        )?;

        tx_without_sig.serialize_content()
    } else {
        transaction.serialize_content()
    };

    if !sig_proof.verify(&tx) {
        error!(
            "Invalid proof. The offending transaction is the following:\n{:?}",
            transaction
        );
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
        error!("Verification of the proof of knowledge for a BLS key failed! For the following BLS public key:\n{:?}",
            validator_key);
        return Err(TransactionError::InvalidData);
    }

    Ok(())
}
