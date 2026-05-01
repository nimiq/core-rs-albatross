use nimiq_primitives::coin::Coin;
use nimiq_serde::Serialize;
use nimiq_transaction::account::staking_contract::{
    IncomingStakingTransactionData, OutgoingStakingTransactionData,
};
use wasm_bindgen::prelude::*;

use crate::common::{
    address::{Address, OptionalAddress},
    signature_proof::SignatureProof,
};

/// The StakingDataBuilder class provides helper methods to easily create staking transaction data.
/// To decode the data into plain objects, use `StakingContract.dataToPlain()`.
#[wasm_bindgen]
pub struct StakingDataBuilder;

#[wasm_bindgen]
impl StakingDataBuilder {
    /// Creates staking transaction data for creating a staker.
    ///
    /// Note: The created data contains an empty signature proof. To add a valid proof:
    /// 1. Set the data on the transaction as `tx.data`
    /// 2. Create a signature proof over the transaction with the staker's keypair
    /// 3. Use `StakingDataBuilder.setProof(tx.data, proof)` to set the created proof on the staking data
    /// 4. Set the updated staking data back on the transaction as `tx.data`
    #[wasm_bindgen(js_name = createStaker)]
    pub fn create_staker(delegation: &OptionalAddress) -> Result<Vec<u8>, JsError> {
        let delegation = wasm_bindgen_derive::try_from_js_option::<Address>(delegation)
            .map_err(|err| JsError::new(&err))?;

        let data = IncomingStakingTransactionData::CreateStaker {
            delegation: delegation.map(|addr| addr.take_native()),
            proof: nimiq_transaction::SignatureProof::default(),
        }
        .serialize_to_vec();

        Ok(data)
    }

    /// Creates staking transaction data for adding stake to a staker.
    ///
    /// Note: This data does not need to be signed seperately and can be used as-is.
    #[wasm_bindgen(js_name = addStake)]
    pub fn add_stake(staker_address: &Address) -> Vec<u8> {
        IncomingStakingTransactionData::AddStake {
            staker_address: staker_address.native_cloned(),
        }
        .serialize_to_vec()
    }

    /// Creates staking transaction data for updating a staker.
    ///
    /// Note: The created data contains an empty signature proof. To add a valid proof:
    /// 1. Set the data on the transaction as `tx.data`
    /// 2. Create a signature proof over the transaction with the staker's keypair
    /// 3. Use `StakingDataBuilder.setProof(tx.data, proof)` to set the created proof on the staking data
    /// 4. Set the updated staking data back on the transaction as `tx.data`
    #[wasm_bindgen(js_name = updateStaker)]
    pub fn update_staker(
        new_delegation: &OptionalAddress,
        reactivate_all_stake: bool,
    ) -> Result<Vec<u8>, JsError> {
        let new_delegation = wasm_bindgen_derive::try_from_js_option::<Address>(new_delegation)
            .map_err(|err| JsError::new(&err))?;

        let data = IncomingStakingTransactionData::UpdateStaker {
            new_delegation: new_delegation.map(|addr| addr.take_native()),
            reactivate_all_stake,
            proof: nimiq_transaction::SignatureProof::default(),
        }
        .serialize_to_vec();

        Ok(data)
    }

    /// Creates staking transaction data for setting the active stake of a staker.
    ///
    /// Note: The created data contains an empty signature proof. To add a valid proof:
    /// 1. Set the data on the transaction as `tx.data`
    /// 2. Create a signature proof over the transaction with the staker's keypair
    /// 3. Use `StakingDataBuilder.setProof(tx.data, proof)` to set the created proof on the staking data
    /// 4. Set the updated staking data back on the transaction as `tx.data`
    ///
    /// Throws when the number given for `new_active_balance` does not fit within a Coin value.
    #[wasm_bindgen(js_name = setActiveStake)]
    pub fn set_active_stake(new_active_balance: u64) -> Result<Vec<u8>, JsError> {
        let data = IncomingStakingTransactionData::SetActiveStake {
            new_active_balance: Coin::try_from(new_active_balance)?,
            proof: nimiq_transaction::SignatureProof::default(),
        }
        .serialize_to_vec();

        Ok(data)
    }

    /// Creates staking transaction data for retiring stake.
    ///
    /// Note: The created data contains an empty signature proof. To add a valid proof:
    /// 1. Set the data on the transaction as `tx.data`
    /// 2. Create a signature proof over the transaction with the staker's keypair
    /// 3. Use `StakingDataBuilder.setProof(tx.data, proof)` to set the created proof on the staking data
    /// 4. Set the updated staking data back on the transaction as `tx.data`
    ///
    /// Throws when the number given for `retire_stake` does not fit within a Coin value.
    #[wasm_bindgen(js_name = retireStake)]
    pub fn retire_stake(retire_stake: u64) -> Result<Vec<u8>, JsError> {
        let data = IncomingStakingTransactionData::RetireStake {
            retire_stake: Coin::try_from(retire_stake)?,
            proof: nimiq_transaction::SignatureProof::default(),
        }
        .serialize_to_vec();

        Ok(data)
    }

    /// Sets the signature proof on the provided staking transaction data.
    #[wasm_bindgen(js_name = setProof)]
    pub fn set_proof(data: &[u8], proof: &SignatureProof) -> Result<Vec<u8>, JsError> {
        IncomingStakingTransactionData::set_signature_on_data(data, proof.native_cloned())
            .map_err(|e| JsError::new(&e.to_string()))
    }

    /// Creates staking transaction data for removing stake from the staking contract.
    ///
    /// Attention: This is used as `senderData` in a transaction with the staking contract as the sender and the staker as signer.
    #[wasm_bindgen(js_name = removeStake)]
    pub fn remove_stake() -> Vec<u8> {
        OutgoingStakingTransactionData::RemoveStake.serialize_to_vec()
    }
}

#[cfg(test)]
mod tests {
    use nimiq_primitives::coin::Coin;
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::StakingDataBuilder;

    #[wasm_bindgen_test]
    fn set_active_stake_validates_coin_range() {
        assert!(StakingDataBuilder::set_active_stake(0).is_ok());
        assert!(StakingDataBuilder::set_active_stake(1).is_ok());
        assert!(StakingDataBuilder::set_active_stake(Coin::MAX_SAFE_VALUE).is_ok());

        assert!(StakingDataBuilder::set_active_stake(Coin::MAX_SAFE_VALUE + 1).is_err());
        assert!(StakingDataBuilder::set_active_stake(u64::MAX).is_err());
    }

    #[wasm_bindgen_test]
    fn retire_stake_validates_coin_range() {
        assert!(StakingDataBuilder::retire_stake(0).is_ok());
        assert!(StakingDataBuilder::retire_stake(1).is_ok());
        assert!(StakingDataBuilder::retire_stake(Coin::MAX_SAFE_VALUE).is_ok());

        assert!(StakingDataBuilder::retire_stake(Coin::MAX_SAFE_VALUE + 1).is_err());
        assert!(StakingDataBuilder::retire_stake(u64::MAX).is_err());
    }
}
