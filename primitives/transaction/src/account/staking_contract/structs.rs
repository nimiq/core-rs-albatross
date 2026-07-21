use nimiq_bls::{CompressedPublicKey as BlsPublicKey, CompressedSignature as BlsSignature};
use nimiq_hash::Blake2bHash;
use nimiq_keys::{Address, Ed25519PublicKey as SchnorrPublicKey};
use nimiq_primitives::{
    coin::Coin,
    policy::{upgrades, Policy},
};
use nimiq_serde::{Deserialize, DeserializeError, Serialize, SerializedMaxSize};

use crate::{SignatureProof, Transaction, TransactionError};

/// We need to distinguish two types of transactions:
/// 1. Incoming transactions. The type of transaction, parameters and proof are given in the `data` field of this transaction.
///    Supported incoming transactions are:
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
///
/// 2. Outgoing transactions. The type of transaction, parameters and proof are given in the `proof` field of this transaction.
///    Supported outgoing transactions are:
///     - Validator
///         * Delete
///     - Staker
///         * RemoveStake
///
/// It is important to note that all `signature` fields contain the signature
/// over the complete transaction with the `signature` field set to `Default::default()`.
/// The field is populated only after computing the signature.
#[derive(Clone, Debug, Serialize, Deserialize, SerializedMaxSize)]
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
        retire_stake: Coin,
        proof: SignatureProof,
    },
    SetSignalData {
        validator_address: Address,
        update: SignalDataUpdate,
        // This proof is signed with the validator warm key.
        proof: SignatureProof,
    },
}

/// Specifies how a [`SetSignalData`](IncomingStakingTransactionData::SetSignalData) transaction
/// updates a validator's `signal_data` field.
#[derive(Clone, Debug, Serialize, Deserialize, SerializedMaxSize)]
#[repr(u8)]
pub enum SignalDataUpdate {
    /// Replaces the *entire* `signal_data` field. `None` clears it (the only way to reach a `None`
    /// signal data); `Some(hash)` sets it verbatim. This overwrites any previously signaled version.
    Full(Option<Blake2bHash>),
    /// Sets *only* the protocol-version bytes (the first two bytes, big-endian) to `version`,
    /// preserving the rest of `signal_data`. Unlike [`SignalDataUpdate::Full`], this is a
    /// non-destructive partial update and always yields a present (`Some`) value — clearing the
    /// whole field is a `Full` operation. Pass `0` to zero the version bytes.
    Version(u16),
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
                | IncomingStakingTransactionData::SetSignalData { .. }
        )
    }

    pub fn parse(transaction: &Transaction) -> Result<Self, TransactionError> {
        Ok(Self::deserialize_all(&transaction.recipient_data)?)
    }

    pub fn verify(
        &self,
        transaction: &Transaction,
        protocol_version: u16,
    ) -> Result<(), TransactionError> {
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
                verify_transaction_signature(transaction, proof)?
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
                match (new_voting_key, new_proof_of_knowledge) {
                    (Some(key), Some(pok)) => verify_proof_of_knowledge(key, pok)?,
                    (Some(_), None) => return Err(TransactionError::InvalidData),
                    (None, Some(_)) => return Err(TransactionError::InvalidData),
                    (None, None) => {} // no key change, nothing to verify
                }

                // Check that the signature is correct.
                verify_transaction_signature(transaction, proof)?
            }
            IncomingStakingTransactionData::DeactivateValidator { proof, .. } => {
                // Check that the signature is correct.
                verify_transaction_signature(transaction, proof)?
            }
            IncomingStakingTransactionData::ReactivateValidator { proof, .. } => {
                // Check that the signature is correct.
                verify_transaction_signature(transaction, proof)?
            }
            IncomingStakingTransactionData::RetireValidator { proof, .. } => {
                // Check that the signature is correct.
                verify_transaction_signature(transaction, proof)?
            }
            IncomingStakingTransactionData::CreateStaker { proof, .. } => {
                // Check that stake is at least minimum stake.
                if transaction.value < Coin::from_u64_unchecked(Policy::MINIMUM_STAKE) {
                    warn!("Can't create a staker with less than minimum stake. The offending transaction is the following:\n{:?}", transaction);
                    return Err(TransactionError::InvalidValue);
                }

                // Check that the signature is correct.
                verify_transaction_signature(transaction, proof)?
            }
            IncomingStakingTransactionData::AddStake { .. } => {
                // Adding stake should be at least greater than 0.
                if transaction.value.is_zero() {
                    warn!("Add stake transactions must have positive value. The offending transaction is the following:\n{:?}", transaction);
                    return Err(TransactionError::ZeroValue);
                }

                // No more checks needed.
            }
            IncomingStakingTransactionData::UpdateStaker { proof, .. } => {
                // Check that the signature is correct.
                verify_transaction_signature(transaction, proof)?
            }
            IncomingStakingTransactionData::SetActiveStake { proof, .. } => {
                // Check that the signature is correct.
                verify_transaction_signature(transaction, proof)?
            }
            IncomingStakingTransactionData::RetireStake {
                proof,
                retire_stake,
            } => {
                // Check that retire is greater than 0.
                if retire_stake.is_zero() {
                    warn!("Retire stake transactions retire a non-zero amount of stake. The offending transaction is the following:\n{:?}", transaction);
                    return Err(TransactionError::ZeroValue);
                }

                // Check that the signature is correct.
                verify_transaction_signature(transaction, proof)?
            }
            IncomingStakingTransactionData::SetSignalData { proof, .. } => {
                // Setting the signal data with the warm key is only allowed once the protocol has
                // upgraded to the version that introduces this transaction. Before that the
                // transaction must be rejected, otherwise nodes that have not upgraded yet would
                // diverge.
                if protocol_version < upgrades::v2::WARM_KEY_SIGNALING {
                    warn!(
                        min_protocol_version = upgrades::v2::WARM_KEY_SIGNALING,
                        protocol_version,
                        ?transaction,
                        "SetSignalData transactions are not allowed before the warm-key signaling upgrade"
                    );
                    return Err(TransactionError::InvalidForVersion);
                }

                // Check that the signature is correct.
                verify_transaction_signature(transaction, proof)?
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
            | IncomingStakingTransactionData::RetireStake { proof, .. }
            | IncomingStakingTransactionData::SetSignalData { proof, .. } => {
                *proof = signature_proof;
            }
            IncomingStakingTransactionData::AddStake { .. } => {}
        }
    }

    pub fn set_signature_on_data(
        data: &[u8],
        signature_proof: SignatureProof,
    ) -> Result<Vec<u8>, DeserializeError> {
        let mut data = IncomingStakingTransactionData::deserialize_from_vec(data)?;
        data.set_signature(signature_proof);
        Ok(data.serialize_to_vec())
    }
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize, SerializedMaxSize)]
#[repr(u8)]
pub enum OutgoingStakingTransactionData {
    DeleteValidator,
    RemoveStake,
}

impl OutgoingStakingTransactionData {
    pub fn parse(transaction: &Transaction) -> Result<Self, TransactionError> {
        Ok(Self::deserialize_all(&transaction.sender_data)?)
    }
}

pub fn verify_transaction_signature(
    transaction: &Transaction,
    sig_proof: &SignatureProof,
) -> Result<(), TransactionError> {
    // Reject the all-zero (default) public key as a signer. It is a verification wildcard
    // (see `SignatureProof::is_default_public_key`)
    if sig_proof.is_default_public_key() {
        warn!(
            ?transaction,
            "Rejecting staking transaction signed with the default (all-zero) public key",
        );
        return Err(TransactionError::InvalidProof);
    }

    // If we are verifying the signature on an incoming transaction, then we need to reset the
    // signature field first.
    let tx = {
        let mut tx_without_sig = transaction.clone();

        tx_without_sig.recipient_data = IncomingStakingTransactionData::set_signature_on_data(
            &tx_without_sig.recipient_data,
            SignatureProof::default(),
        )?;

        tx_without_sig.serialize_content()
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
