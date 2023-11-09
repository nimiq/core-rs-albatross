use nimiq_keys::Address;
use nimiq_primitives::coin::Coin;
use serde::{Deserialize, Serialize};

#[derive(Clone, Eq, PartialEq, Debug, Serialize, Deserialize)]
#[repr(C)]
pub struct RewardTransaction {
    /// The validator address of the rewarded validator.
    pub validator_address: Address,
    /// The address the reward was paid out to.
    pub recipient: Address,
    /// The reward amount.
    pub value: Coin,
}
