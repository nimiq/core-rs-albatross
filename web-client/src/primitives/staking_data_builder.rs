use nimiq_primitives::coin::Coin;
use nimiq_serde::Serialize;
use nimiq_transaction::account::staking_contract::IncomingStakingTransactionData;
use wasm_bindgen::prelude::*;

use crate::common::{
    address::{Address, OptionalAddress},
    signature_proof::SignatureProof,
};

#[wasm_bindgen]
pub struct StakingDataBuilder;

#[wasm_bindgen]
impl StakingDataBuilder {
    #[wasm_bindgen(js_name = createStaker)]
    pub fn create_staker(delegation: &OptionalAddress) -> Result<Vec<u8>, JsError> {
        let delegation = wasm_bindgen_derive::try_from_js_option::<Address>(delegation)
            .map_err(|err| JsError::new(&err))?;

        let data = IncomingStakingTransactionData::CreateStaker {
            delegation: delegation.map(|addr| addr.native()),
            proof: nimiq_transaction::SignatureProof::default(),
        }
        .serialize_to_vec();

        Ok(data)
    }

    #[wasm_bindgen(js_name = addStake)]
    pub fn add_stake(staker_address: &Address) -> Vec<u8> {
        IncomingStakingTransactionData::AddStake {
            staker_address: staker_address.native(),
        }
        .serialize_to_vec()
    }

    #[wasm_bindgen(js_name = updateStaker)]
    pub fn update_staker(
        new_delegation: &OptionalAddress,
        reactivate_all_stake: bool,
    ) -> Result<Vec<u8>, JsError> {
        let new_delegation = wasm_bindgen_derive::try_from_js_option::<Address>(new_delegation)
            .map_err(|err| JsError::new(&err))?;

        let data = IncomingStakingTransactionData::UpdateStaker {
            new_delegation: new_delegation.map(|addr| addr.native()),
            reactivate_all_stake,
            proof: nimiq_transaction::SignatureProof::default(),
        }
        .serialize_to_vec();

        Ok(data)
    }

    #[wasm_bindgen(js_name = setActiveStake)]
    pub fn set_active_stake(new_active_balance: u64) -> Vec<u8> {
        IncomingStakingTransactionData::SetActiveStake {
            new_active_balance: Coin::from_u64_unchecked(new_active_balance),
            proof: nimiq_transaction::SignatureProof::default(),
        }
        .serialize_to_vec()
    }

    #[wasm_bindgen(js_name = retireStaker)]
    pub fn retire_staker(retire_stake: u64) -> Vec<u8> {
        IncomingStakingTransactionData::RetireStake {
            retire_stake: Coin::from_u64_unchecked(retire_stake),
            proof: nimiq_transaction::SignatureProof::default(),
        }
        .serialize_to_vec()
    }

    #[wasm_bindgen(js_name = setProof)]
    pub fn set_proof(data: &[u8], proof: &SignatureProof) -> Result<Vec<u8>, JsError> {
        IncomingStakingTransactionData::set_signature_on_data(data, proof.native())
            .map_err(|e| JsError::new(&e.to_string()))
    }
}
