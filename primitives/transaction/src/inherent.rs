use crate::reward::RewardTransaction;
use beserial::{Deserialize, Serialize};
use nimiq_keys::Address;
use nimiq_primitives::coin::Coin;
use nimiq_primitives::policy;
use nimiq_primitives::slots::SlashedSlot;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[repr(u8)]
pub enum Inherent {
    Reward { target: Address, value: Coin },
    Slash { slot: SlashedSlot },
    FinalizeBatch,
    FinalizeEpoch,
}

impl Inherent {
    pub fn target(&self) -> &Address {
        match self {
            Inherent::Reward { target, .. } => target,
            Inherent::Slash { .. } | Inherent::FinalizeBatch | Inherent::FinalizeEpoch => {
                &policy::STAKING_CONTRACT_ADDRESS
            }
        }
    }
}

impl From<&RewardTransaction> for Inherent {
    fn from(tx: &RewardTransaction) -> Self {
        Self {
            ty: InherentType::Reward,
            target: tx.recipient.clone(),
            value: tx.value,
            data: vec![],
        }
    }
}
