use nimiq_keys::Address;
use nimiq_primitives::coin::Coin;
use serde::{Deserialize, Serialize};

#[derive(Clone, Eq, PartialEq, Debug, Serialize, Deserialize)]
#[repr(C)]
pub struct RewardTransaction {
    pub validator_address: Address,
    pub recipient: Address,
    pub value: Coin,
}
