use nimiq_serde::Deserialize;
use nimiq_transaction::account::staking_contract::IncomingStakingTransactionData;
use wasm_bindgen::prelude::*;

#[cfg(feature = "primitives")]
use crate::common::transaction::{PlainTransactionProofType, PlainTransactionRecipientDataType};
use crate::common::{
    signature_proof::SignatureProof,
    transaction::{
        PlainAddStakeData, PlainCreateStakerData, PlainCreateValidatorData, PlainRawData,
        PlainRetireStakeData, PlainSetActiveStakeData, PlainSetSignalDataData,
        PlainTransactionProof, PlainTransactionRecipientData, PlainUpdateStakerData,
        PlainUpdateValidatorData, PlainValidatorData,
    },
};

/// Utility class providing methods to parse Staking Contract transaction data and proofs.
#[wasm_bindgen]
pub struct StakingContract;

#[cfg(feature = "primitives")]
#[wasm_bindgen]
impl StakingContract {
    /// Parses the data of a Staking Contract incoming transaction into a plain object.
    #[wasm_bindgen(js_name = dataToPlain)]
    pub fn data_to_plain(data: &[u8]) -> Result<PlainTransactionRecipientDataType, JsError> {
        let plain = StakingContract::parse_data(data)?;
        Ok(serde_wasm_bindgen::to_value(&plain)?.into())
    }

    /// Parses the proof of a Staking Contract outgoing transaction into a plain object.
    #[wasm_bindgen(js_name = proofToPlain)]
    pub fn proof_to_plain(proof: &[u8]) -> Result<PlainTransactionProofType, JsError> {
        let plain = StakingContract::parse_proof(proof)?;
        Ok(serde_wasm_bindgen::to_value(&plain)?.into())
    }
}

impl StakingContract {
    pub fn parse_data(bytes: &[u8]) -> Result<PlainTransactionRecipientData, JsError> {
        let data = IncomingStakingTransactionData::deserialize_all(bytes)?;
        Ok(match data {
            IncomingStakingTransactionData::CreateStaker {
                delegation,
                proof: _proof,
            } => PlainTransactionRecipientData::CreateStaker(PlainCreateStakerData {
                raw: hex::encode(bytes),
                delegation: delegation.map(|address| address.to_user_friendly_address()),
            }),
            IncomingStakingTransactionData::AddStake { staker_address } => {
                PlainTransactionRecipientData::AddStake(PlainAddStakeData {
                    raw: hex::encode(bytes),
                    staker: staker_address.to_user_friendly_address(),
                })
            }
            IncomingStakingTransactionData::UpdateStaker {
                new_delegation,
                reactivate_all_stake,
                proof: _proof,
            } => PlainTransactionRecipientData::UpdateStaker(PlainUpdateStakerData {
                raw: hex::encode(bytes),
                new_delegation: new_delegation.map(|address| address.to_user_friendly_address()),
                reactivate_all_stake,
            }),
            IncomingStakingTransactionData::CreateValidator {
                signing_key,
                voting_key,
                reward_address,
                signal_data,
                proof_of_knowledge,
                proof: _proof,
            } => PlainTransactionRecipientData::CreateValidator(PlainCreateValidatorData {
                raw: hex::encode(bytes),
                signing_key: signing_key.to_hex(),
                voting_key: voting_key.to_hex(),
                reward_address: reward_address.to_user_friendly_address(),
                signal_data: signal_data.map(hex::encode),
                proof_of_knowledge: proof_of_knowledge.to_hex(),
            }),
            IncomingStakingTransactionData::UpdateValidator {
                new_signing_key,
                new_voting_key,
                new_reward_address,
                new_signal_data,
                new_proof_of_knowledge,
                proof: _proof,
            } => PlainTransactionRecipientData::UpdateValidator(PlainUpdateValidatorData {
                raw: hex::encode(bytes),
                new_signing_key: new_signing_key.map(|signing_key| signing_key.to_hex()),
                new_voting_key: new_voting_key.map(|voting_key| voting_key.to_hex()),
                new_reward_address: new_reward_address
                    .map(|reward_address| reward_address.to_user_friendly_address()),
                new_signal_data: new_signal_data.map(|signal_data| signal_data.map(hex::encode)),
                new_proof_of_knowledge: new_proof_of_knowledge
                    .map(|proof_of_knowledge| proof_of_knowledge.to_hex()),
            }),
            IncomingStakingTransactionData::DeactivateValidator {
                validator_address,
                proof: _proof,
            } => PlainTransactionRecipientData::DeactivateValidator(PlainValidatorData {
                raw: hex::encode(bytes),
                validator: validator_address.to_user_friendly_address(),
            }),
            IncomingStakingTransactionData::ReactivateValidator {
                validator_address,
                proof: _proof,
            } => PlainTransactionRecipientData::ReactivateValidator(PlainValidatorData {
                raw: hex::encode(bytes),
                validator: validator_address.to_user_friendly_address(),
            }),
            IncomingStakingTransactionData::RetireValidator { proof: _proof } => {
                PlainTransactionRecipientData::RetireValidator(PlainRawData {
                    raw: hex::encode(bytes),
                })
            }
            IncomingStakingTransactionData::SetActiveStake {
                new_active_balance,
                proof: _proof,
            } => PlainTransactionRecipientData::SetActiveStake(PlainSetActiveStakeData {
                raw: hex::encode(bytes),
                new_active_balance: new_active_balance.into(),
            }),
            IncomingStakingTransactionData::RetireStake {
                retire_stake,
                proof: _proof,
            } => PlainTransactionRecipientData::RetireStake(PlainRetireStakeData {
                raw: hex::encode(bytes),
                retire_stake: retire_stake.into(),
            }),
            IncomingStakingTransactionData::SetSignalData {
                validator_address,
                new_signal_data,
                proof: _proof,
            } => PlainTransactionRecipientData::SetSignalData(PlainSetSignalDataData {
                raw: hex::encode(bytes),
                validator: validator_address.to_user_friendly_address(),
                new_signal_data: new_signal_data.map(hex::encode),
            }),
        })
    }

    pub fn parse_proof(bytes: &[u8]) -> Result<PlainTransactionProof, JsError> {
        let proof = SignatureProof::deserialize(bytes)?;
        Ok(proof.to_plain_transaction_proof())
    }
}
