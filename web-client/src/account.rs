use tsify::Tsify;
use wasm_bindgen::prelude::*;

#[derive(serde::Serialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct PlainBasicAccount {
    balance: u64,
}

#[derive(serde::Serialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct PlainVestingAccount {
    balance: u64,
    owner: String,
    start_time: u64,
    time_step: u64,
    step_amount: u64,
    total_amount: u64,
}

#[derive(serde::Serialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct PlainHtlcAccount {
    balance: u64,
    sender: String,
    recipient: String,
    hash_algorithm: String,
    hash_root: String,
    hash_count: u8,
    timeout: u64,
    total_amount: u64,
}

#[derive(serde::Serialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct PlainStakingAccount {
    balance: u64,
    active_validators: Vec<(String, u64)>,
    parked_set: Vec<String>,
    current_lost_rewards: String,
    previous_lost_rewards: String,
    current_disabled_slots: Vec<(String, Vec<u16>)>,
    previous_disabled_slots: Vec<(String, Vec<u16>)>,
}

#[derive(serde::Serialize, Tsify)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum PlainAccount {
    Basic(PlainBasicAccount),
    Vesting(PlainVestingAccount),
    Htlc(PlainHtlcAccount),
    Staking(PlainStakingAccount),
}

impl PlainAccount {
    pub fn from_native(account: &nimiq_account::Account) -> PlainAccount {
        match account {
            nimiq_account::Account::Basic(acc) => PlainAccount::Basic(PlainBasicAccount {
                balance: acc.balance.into(),
            }),
            nimiq_account::Account::Vesting(acc) => PlainAccount::Vesting(PlainVestingAccount {
                balance: acc.balance.into(),
                owner: acc.owner.to_user_friendly_address(),
                start_time: acc.start_time,
                time_step: acc.time_step,
                step_amount: acc.step_amount.into(),
                total_amount: acc.total_amount.into(),
            }),
            nimiq_account::Account::HTLC(acc) => PlainAccount::Htlc(PlainHtlcAccount {
                balance: acc.balance.into(),
                sender: acc.sender.to_user_friendly_address(),
                recipient: acc.recipient.to_user_friendly_address(),
                hash_algorithm: match acc.hash_algorithm {
                    nimiq_transaction::account::htlc_contract::HashAlgorithm::Blake2b => {
                        "blake2b".to_string()
                    }
                    nimiq_transaction::account::htlc_contract::HashAlgorithm::Sha256 => {
                        "sha256".to_string()
                    }
                },
                hash_root: acc.hash_root.to_hex(),
                hash_count: acc.hash_count,
                timeout: acc.timeout,
                total_amount: acc.total_amount.into(),
            }),
            nimiq_account::Account::Staking(acc) => PlainAccount::Staking(PlainStakingAccount {
                balance: acc.balance.into(),
                active_validators: acc
                    .active_validators
                    .iter()
                    .map(|(address, balance)| {
                        (
                            address.to_user_friendly_address(),
                            balance.to_owned().into(),
                        )
                    })
                    .collect(),
                parked_set: acc
                    .parked_set
                    .iter()
                    .map(|address| address.to_user_friendly_address())
                    .collect(),
                current_lost_rewards: acc.current_lost_rewards.to_string(),
                previous_lost_rewards: acc.previous_lost_rewards.to_string(),
                current_disabled_slots: acc
                    .current_disabled_slots
                    .iter()
                    .map(|(address, slots)| {
                        (
                            address.to_user_friendly_address(),
                            slots.iter().cloned().collect(),
                        )
                    })
                    .collect(),
                previous_disabled_slots: acc
                    .previous_disabled_slots
                    .iter()
                    .map(|(address, slots)| {
                        (
                            address.to_user_friendly_address(),
                            slots.iter().cloned().collect(),
                        )
                    })
                    .collect(),
            }),
        }
    }
}

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "PlainAccount")]
    pub type PlainAccountType;

    #[wasm_bindgen(typescript_type = "PlainAccount[]")]
    pub type PlainAccountArrayType;
}
