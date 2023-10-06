use std::ops::Range;

use nimiq_hash_derive::SerializeContent;
use nimiq_keys::Address;
use nimiq_primitives::{
    coin::Coin,
    policy::Policy,
    slots_allocation::{JailedValidator, PenalizedSlot},
};
use nimiq_serde::{Deserialize, Serialize};

use crate::reward::RewardTransaction;

/// An inherent is an intrinsic operation of the blockchain. It is derived from the blocks content.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, SerializeContent, Deserialize)]
#[repr(u8)]
pub enum Inherent {
    /// A reward is given for elected slots that did not get punished.
    Reward {
        validator_address: Address,
        target: Address,
        value: Coin,
    },
    /// Penalties are the consequence of delaying blocks. They only affect the reward of a single slot.
    /// The validator gets deactivated as a consequence.
    Penalize { slot: PenalizedSlot },
    /// Jailing is a consequence of malicious misbehavior, such as forks, double voting, etc.
    /// This affects all validator slots and also penalizes all of them. Additionally, the validator
    /// gets jailed as a consequence.
    Jail {
        jailed_validator: JailedValidator,
        new_epoch_slot_range: Option<Range<u16>>,
    },
    /// Called in every macro block finalization. When we finish an epoch, there are both a
    /// `FinalizeBatch` and `FinalizeEpoch` emitted.
    FinalizeBatch,
    /// Emitted only on epoch finalization.
    FinalizeEpoch,
}

impl Inherent {
    pub fn target(&self) -> &Address {
        match self {
            Inherent::Reward { target, .. } => target,
            Inherent::Penalize { .. }
            | Inherent::Jail { .. }
            | Inherent::FinalizeBatch
            | Inherent::FinalizeEpoch => &Policy::STAKING_CONTRACT_ADDRESS,
        }
    }
}

impl From<&RewardTransaction> for Inherent {
    fn from(tx: &RewardTransaction) -> Self {
        Self::Reward {
            validator_address: tx.validator_address.clone(),
            target: tx.recipient.clone(),
            value: tx.value,
        }
    }
}
