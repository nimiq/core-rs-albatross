use nimiq_bls::{CompressedPublicKey as BlsPublicKey, CompressedSignature as BlsSignature};
use nimiq_hash::Blake2bHash;
use nimiq_keys::{Address, PublicKey as SchnorrPublicKey};
use nimiq_primitives::{coin::Coin, policy::Policy};
use nimiq_serde::{Deserialize, DeserializeError, Serialize};

use crate::{SignatureProof, Transaction, TransactionError};

/// We need to distinguish two types of transactions:
/// 1. Incoming transactions, which include:
///     - Validator
///         * Create
///         * Update
///         * Deactivate
///         * Reactivate
///         * Retire
///     - Staker
///         * Create
///         * Update
///         * AddStake
///     The type of transaction, parameters and proof are given in the data field of the transaction.
/// 2. Outgoing transactions, which include:
///     - Validator
///         * Delete
///     - Staker
///         * RemoveStake
///     The type of transaction, parameters and proof are given in the proof field of the transaction.
///
/// It is important to note that all `signature` fields contain the signature
/// over the complete transaction with the `signature` field set to `Default::default()`.
/// The field is populated only after computing the signature.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum IncomingStakingTransactionData {
    CreateValidator {
        signing_key: SchnorrPublicKey,
        voting_key: BlsPublicKey,
        reward_address: Address,
        signal_data: Option<Blake2bHash>,
        proof_of_knowledge: BlsSignature,
        // This proof is signed with the validator cold key, which will become the validator address.
        proof: SignatureProof,
    },
    UpdateValidator {
        new_signing_key: Option<SchnorrPublicKey>,
        new_voting_key: Option<BlsPublicKey>,
        new_reward_address: Option<Address>,
        new_signal_data: Option<Option<Blake2bHash>>,
        new_proof_of_knowledge: Option<BlsSignature>,
        // This proof is signed with the validator cold key.
        proof: SignatureProof,
    },
    DeactivateValidator {
        validator_address: Address,
        // This proof is signed with the validator warm key.
        proof: SignatureProof,
    },
    ReactivateValidator {
        validator_address: Address,
        // This proof is signed with the validator warm key.
        proof: SignatureProof,
    },
    RetireValidator {
        // This proof is signed with the validator cold key.
        proof: SignatureProof,
    },
    CreateStaker {
        delegation: Option<Address>,
        proof: SignatureProof,
    },
    AddStake {
        staker_address: Address,
    },
    UpdateStaker {
        new_delegation: Option<Address>,
        reactivate_all_stake: bool,
        proof: SignatureProof,
    },
    SetActiveStake {
        new_active_balance: Coin,
        proof: SignatureProof,
    },
    RetireStake {
        value: Coin,
        proof: SignatureProof,
    },
}

impl IncomingStakingTransactionData {
    pub fn is_signaling(&self) -> bool {
        matches!(
            self,
            IncomingStakingTransactionData::UpdateValidator { .. }
                | IncomingStakingTransactionData::DeactivateValidator { .. }
                | IncomingStakingTransactionData::ReactivateValidator { .. }
                | IncomingStakingTransactionData::RetireValidator { .. }
                | IncomingStakingTransactionData::UpdateStaker { .. }
                | IncomingStakingTransactionData::SetActiveStake { .. }
                | IncomingStakingTransactionData::RetireStake { .. }
        )
    }

    pub fn parse(transaction: &Transaction) -> Result<Self, TransactionError> {
        full_parse(&transaction.recipient_data[..])
    }

    pub fn verify(&self, transaction: &Transaction) -> Result<(), TransactionError> {
        match self {
            IncomingStakingTransactionData::CreateValidator {
                voting_key,
                proof_of_knowledge,
                proof,
                ..
            } => {
                // Validators must be created with exactly the validator deposit amount.
                if transaction.value != Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT) {
                    warn!("Validator stake value different from VALIDATOR_DEPOSIT. The offending transaction is the following:\n{:?}", transaction);
                    return Err(TransactionError::InvalidValue);
                }

                // Check proof of knowledge.
                verify_proof_of_knowledge(voting_key, proof_of_knowledge)?;

                // Check that the signature is correct.
                verify_transaction_signature(transaction, proof, true)?
            }
            IncomingStakingTransactionData::UpdateValidator {
                new_signing_key,
                new_voting_key,
                new_reward_address,
                new_signal_data,
                new_proof_of_knowledge,
                proof,
            } => {
                // Do not allow updates without any effect.
                if new_signing_key.is_none()
                    && new_voting_key.is_none()
                    && new_reward_address.is_none()
                    && new_signal_data.is_none()
                {
                    warn!("Signaling update transactions must actually update something. The offending transaction is the following:\n{:?}", transaction);
                    return Err(TransactionError::InvalidData);
                }

                // Check proof of knowledge, if necessary.
                if let (Some(new_voting_key), Some(new_proof_of_knowledge)) =
                    (new_voting_key, new_proof_of_knowledge)
                {
                    verify_proof_of_knowledge(new_voting_key, new_proof_of_knowledge)?;
                }

                // Check that the signature is correct.
                verify_transaction_signature(transaction, proof, true)?
            }
            IncomingStakingTransactionData::DeactivateValidator { proof, .. } => {
                // Check that the signature is correct.
                verify_transaction_signature(transaction, proof, true)?
            }
            IncomingStakingTransactionData::ReactivateValidator { proof, .. } => {
                // Check that the signature is correct.
                verify_transaction_signature(transaction, proof, true)?
            }
            IncomingStakingTransactionData::RetireValidator { proof, .. } => {
                // Check that the signature is correct.
                verify_transaction_signature(transaction, proof, true)?
            }
            IncomingStakingTransactionData::CreateStaker { proof, .. } => {
                // Check that stake is bigger than the minimum stake.
                if transaction.value < Coin::from_u64_unchecked(Policy::MINIMUM_STAKE) {
                    warn!("Can't create a staker with less than minimum stake. The offending transaction is the following:\n{:?}", transaction);
                    return Err(TransactionError::InvalidValue);
                }

                // Check that the signature is correct.
                verify_transaction_signature(transaction, proof, true)?
            }
            IncomingStakingTransactionData::AddStake { .. } => {
                // Adding stake should be at least greater than 0.
                if transaction.value.is_zero() {
                    warn!("Add stake transactions must actually have higher than 0 value. The offending transaction is the following:\n{:?}", transaction);
                    return Err(TransactionError::ZeroValue);
                }

                // No more checks needed.
            }
            IncomingStakingTransactionData::UpdateStaker { proof, .. } => {
                // Check that the signature is correct.
                verify_transaction_signature(transaction, proof, true)?
            }
            IncomingStakingTransactionData::SetActiveStake { proof, .. } => {
                // Check that the signature is correct.
                verify_transaction_signature(transaction, proof, true)?
            }
            IncomingStakingTransactionData::RetireStake { proof, value } => {
                // Check that retire is greater than 0.
                if value.is_zero() {
                    warn!("Retire stake transactions must actually have higher than 0 value. The offending transaction is the following:\n{:?}", transaction);
                    return Err(TransactionError::ZeroValue);
                }

                // Check that the signature is correct.
                verify_transaction_signature(transaction, proof, true)?
            }
        }

        Ok(())
    }

    pub fn set_signature(&mut self, signature_proof: SignatureProof) {
        match self {
            IncomingStakingTransactionData::CreateValidator { proof, .. }
            | IncomingStakingTransactionData::UpdateValidator { proof, .. }
            | IncomingStakingTransactionData::DeactivateValidator { proof, .. }
            | IncomingStakingTransactionData::ReactivateValidator { proof, .. }
            | IncomingStakingTransactionData::RetireValidator { proof, .. }
            | IncomingStakingTransactionData::CreateStaker { proof, .. }
            | IncomingStakingTransactionData::UpdateStaker { proof, .. }
            | IncomingStakingTransactionData::SetActiveStake { proof, .. }
            | IncomingStakingTransactionData::RetireStake { proof, .. } => {
                *proof = signature_proof;
            }
            IncomingStakingTransactionData::AddStake { .. } => {}
        }
    }

    pub fn set_signature_on_data(
        data: &[u8],
        signature_proof: SignatureProof,
    ) -> Result<Vec<u8>, DeserializeError> {
        let mut data: IncomingStakingTransactionData = Deserialize::deserialize_from_vec(data)?;
        data.set_signature(signature_proof);
        Ok(data.serialize_to_vec())
    }
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum OutgoingStakingTransactionData {
    DeleteValidator,
    RemoveStake,
}

impl OutgoingStakingTransactionData {
    pub fn parse(transaction: &Transaction) -> Result<Self, TransactionError> {
        full_parse(&transaction.sender_data[..])
    }
}

pub fn full_parse<T: Deserialize>(data: &[u8]) -> Result<T, TransactionError> {
    let (data, left_over) = T::deserialize_take(data)?;

    // Ensure that transaction data has been fully read.
    if !left_over.is_empty() {
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

        tx_without_sig.recipient_data = IncomingStakingTransactionData::set_signature_on_data(
            &tx_without_sig.recipient_data,
            SignatureProof::default(),
        )?;

        tx_without_sig.serialize_content()
    } else {
        transaction.serialize_content()
    };

    if !sig_proof.verify(&tx) {
        warn!(
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
    voting_key: &BlsPublicKey,
    proof_of_knowledge: &BlsSignature,
) -> Result<(), TransactionError> {
    if !voting_key
        .uncompress()
        .map_err(|_| TransactionError::InvalidData)?
        .verify(
            voting_key,
            &proof_of_knowledge
                .uncompress()
                .map_err(|_| TransactionError::InvalidData)?,
        )
    {
        warn!("Verification of the proof of knowledge for a BLS key failed! For the following BLS public key:\n{:?}",
            voting_key);
        return Err(TransactionError::InvalidData);
    }

    Ok(())
}
