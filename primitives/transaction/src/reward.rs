use nimiq_keys::Address;
use nimiq_primitives::coin::Coin;
use nimiq_serde::{Deserialize, Serialize, SerializedMaxSize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, SerializedMaxSize)]
pub struct RewardTransaction {
    /// The validator address of the rewarded validator.
    pub validator_address: Address,
    /// The address the reward was paid out to.
    pub recipient: Address,
    /// The reward amount.
    pub value: Coin,
}
