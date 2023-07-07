use std::collections::BTreeSet;

use nimiq_bls::CompressedPublicKey as BlsPublicKey;
use nimiq_hash::Blake2bHash;
use nimiq_keys::{Address, PublicKey as SchnorrPublicKey};
use nimiq_primitives::{account::AccountError, coin::Coin};
use nimiq_serde::{Deserialize, Serialize};

use crate::{convert_receipt, AccountReceipt};

/// A collection of receipts for inherents/transactions. This is necessary to be able to revert
/// those inherents/transactions.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct PenalizeReceipt {
    pub newly_deactivated: bool,
    pub newly_disabled: bool,
    pub newly_lost_rewards: bool,
    pub old_jail_release: Option<u32>,
}
convert_receipt!(PenalizeReceipt);

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct UpdateValidatorReceipt {
    pub old_signing_key: SchnorrPublicKey,
    pub old_voting_key: BlsPublicKey,
    pub old_reward_address: Address,
    pub old_signal_data: Option<Blake2bHash>,
}
convert_receipt!(UpdateValidatorReceipt);

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct JailValidatorReceipt {
    pub newly_deactivated: bool,
    pub old_jail_release: Option<u32>,
}
convert_receipt!(JailValidatorReceipt);

impl From<PenalizeReceipt> for JailValidatorReceipt {
    fn from(value: PenalizeReceipt) -> Self {
        Self {
            newly_deactivated: value.newly_deactivated,
            old_jail_release: value.old_jail_release,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct ReactivateValidatorReceipt {
    pub was_inactive_since: u32,
    pub current_epoch_disabled_slots: Option<BTreeSet<u16>>,
}
convert_receipt!(ReactivateValidatorReceipt);

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct RetireValidatorReceipt {
    pub was_active: bool,
}
convert_receipt!(RetireValidatorReceipt);

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct DeleteValidatorReceipt {
    pub signing_key: SchnorrPublicKey,
    pub voting_key: BlsPublicKey,
    pub reward_address: Address,
    pub signal_data: Option<Blake2bHash>,
    pub inactive_since: u32,
    pub jail_release: Option<u32>,
}
convert_receipt!(DeleteValidatorReceipt);

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct StakerReceipt {
    pub delegation: Option<Address>,
}
convert_receipt!(StakerReceipt);

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct SetInactiveStakeReceipt {
    pub old_inactive_release: Option<u32>,
    pub old_active_balance: Coin,
}
convert_receipt!(SetInactiveStakeReceipt);

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct RemoveStakeReceipt {
    pub delegation: Option<Address>,
    pub inactive_release: Option<u32>,
}
convert_receipt!(RemoveStakeReceipt);
