use nimiq_keys::Address;
use nimiq_primitives::coin::Coin;
use serde::{Deserialize, Serialize};

#[derive(Clone, Eq, PartialEq, Debug, Serialize, Deserialize)]
pub struct RewardTransaction {
    /// The validator address of the rewarded validator.
    pub validator_address: Address,
    /// The address the reward was paid out to.
    pub recipient: Address,
    /// The reward amount.
    pub value: Coin,
}

impl RewardTransaction {
    #[allow(clippy::identity_op)]
    pub const SIZE: usize = 0
        + /*validator_address*/ Address::SIZE
        + /*recipient*/ Address::SIZE
        + /*value*/ Coin::SIZE;
}
