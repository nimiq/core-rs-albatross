use std::collections::BTreeSet;

use nimiq_bls::CompressedPublicKey as BlsPublicKey;
use nimiq_collections::BitSet;
use nimiq_hash::Blake2bHash;
use nimiq_keys::{Address, Ed25519PublicKey as SchnorrPublicKey};
use nimiq_primitives::{account::AccountError, coin::Coin};
use nimiq_serde::{Deserialize, Serialize};

use crate::{convert_receipt, AccountReceipt};

/// Penalize receipt for the inherent. This is necessary to be able to revert
/// these inherents.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct PenalizeReceipt {
    /// true if corresponding validator was deactivated by this penalty
    pub newly_deactivated: bool,
    /// true if corresponding slot was punished for the first time
    /// for an offense in the previous batch
    pub newly_punished_previous_batch: bool,
    /// true if corresponding slot was punished for the first time
    /// in the current batch
    pub newly_punished_current_batch: bool,
}
convert_receipt!(PenalizeReceipt);

/// Jail receipt for the inherent. This is necessary to be able to revert
/// these inherents.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct JailReceipt {
    /// true if corresponding validator was deactivated by this jail
    pub newly_deactivated: bool,
    /// the previous batch punished slots before this jail is applied
    pub old_previous_batch_punished_slots: BitSet,
    /// the current batch punished slots of the affected validator before this jail is applied
    pub old_current_batch_punished_slots: Option<BTreeSet<u16>>,
    /// the jailed block height before this jail is applied
    pub old_jailed_from: Option<u32>,
}
convert_receipt!(JailReceipt);

/// Receipt for update validator transactions. This is necessary to be able to revert
/// these transactions.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct UpdateValidatorReceipt {
    /// the signing key before this transaction is applied
    pub old_signing_key: SchnorrPublicKey,
    /// the voting key before this transaction is applied
    pub old_voting_key: BlsPublicKey,
    /// the reward address before this transaction is applied
    pub old_reward_address: Address,
    // the signal data before this transaction is applied
    pub old_signal_data: Option<Blake2bHash>,
}
convert_receipt!(UpdateValidatorReceipt);

/// Receipt for jailing a validator. This is necessary to be able to revert
/// a jail.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct JailValidatorReceipt {
    /// true if corresponding validator was deactivated by this jail
    pub newly_deactivated: bool,
    /// the jail block height before this jail is applied
    pub old_jailed_from: Option<u32>,
}
convert_receipt!(JailValidatorReceipt);

impl From<&JailReceipt> for JailValidatorReceipt {
    fn from(value: &JailReceipt) -> Self {
        Self {
            newly_deactivated: value.newly_deactivated,
            old_jailed_from: value.old_jailed_from,
        }
    }
}

/// Receipt for reactivate validator transactions. This is necessary to be able to revert
/// these transactions.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct ReactivateValidatorReceipt {
    /// the value of `inactive_from` before this transaction is applied
    pub was_inactive_from: u32,
}
convert_receipt!(ReactivateValidatorReceipt);

/// Receipt for retire validator transactions. This is necessary to be able to revert
/// these transactions.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct RetireValidatorReceipt {
    /// true if the validator was retired from an active state
    pub was_active: bool,
}
convert_receipt!(RetireValidatorReceipt);

/// Receipt for delete validator transactions. This is necessary to be able to revert
/// these transactions.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct DeleteValidatorReceipt {
    /// the signing key before this transaction is applied
    pub signing_key: SchnorrPublicKey,
    /// the voting key before this transaction is applied
    pub voting_key: BlsPublicKey,
    /// the reward address before this transaction is applied
    pub reward_address: Address,
    /// the signal data before this transaction is applied
    pub signal_data: Option<Blake2bHash>,
    /// the value of `inactive_from` before this transaction is applied
    pub inactive_from: u32,
    /// the jail release before this transaction is applied
    pub jailed_from: Option<u32>,
}
convert_receipt!(DeleteValidatorReceipt);

/// Receipt for most staker-related transactions. This is necessary to be able to revert
/// these transactions.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct StakerReceipt {
    /// the delegation before this transaction is applied
    pub delegation: Option<Address>,
    /// the active balance before this transaction is applied
    pub active_balance: Coin,
    /// the inactivation block height before this transaction is applied
    pub inactive_from: Option<u32>,
}
convert_receipt!(StakerReceipt);

/// Receipt for set inactive stake transactions. This is necessary to be able to revert
/// these transactions.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct SetInactiveStakeReceipt {
    /// the inactivation block height before this transaction is applied
    pub old_inactive_from: Option<u32>,
    /// the active balance before this transaction is applied
    pub old_active_balance: Coin,
}
convert_receipt!(SetInactiveStakeReceipt);

/// Receipt for remove stake transactions. This is necessary to be able to revert
/// these transactions.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct RemoveStakeReceipt {
    /// the delegation before this transaction is applied
    pub delegation: Option<Address>,
    /// the inactivation block height before this transaction is applied
    pub inactive_from: Option<u32>,
}
convert_receipt!(RemoveStakeReceipt);
