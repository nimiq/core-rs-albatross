use nimiq_keys::Address;
use nimiq_primitives::coin::Coin;
use serde::{Deserialize, Serialize};

#[derive(Clone, Eq, PartialEq, Debug, Serialize, Deserialize)]
#[repr(C)]
pub struct RewardTransaction {
    pub recipient: Address,
    pub value: Coin,
}

impl RewardTransaction {
    pub fn new(recipient: Address, value: Coin) -> Self {
        Self { recipient, value }
    }
}
