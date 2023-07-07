use std::ops::Range;

use nimiq_hash_derive::SerializeContent;
use nimiq_keys::Address;
use nimiq_primitives::{
    coin::Coin,
    policy::Policy,
    slots_allocation::{PenalizedSlot, SlashedValidator},
};
use nimiq_serde::{Deserialize, Serialize};

use crate::reward::RewardTransaction;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, SerializeContent, Deserialize)]
#[repr(u8)]
pub enum Inherent {
    Reward {
        target: Address,
        value: Coin,
    },
    Penalize {
        slot: PenalizedSlot,
    },
    Slash {
        slashed_validator: SlashedValidator,
        new_epoch_slot_range: Option<Range<u16>>,
    },
    FinalizeBatch,
    FinalizeEpoch,
}

impl Inherent {
    pub fn target(&self) -> &Address {
        match self {
            Inherent::Reward { target, .. } => target,
            Inherent::Penalize { .. }
            | Inherent::Slash { .. }
            | Inherent::FinalizeBatch
            | Inherent::FinalizeEpoch => &Policy::STAKING_CONTRACT_ADDRESS,
        }
    }
}

impl From<&RewardTransaction> for Inherent {
    fn from(tx: &RewardTransaction) -> Self {
        Self::Reward {
            target: tx.recipient.clone(),
            value: tx.value,
        }
    }
}
