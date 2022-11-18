use std::collections::BTreeSet;

use beserial::{Deserialize, Serialize};
use nimiq_bls::CompressedPublicKey as BlsPublicKey;
use nimiq_hash::Blake2bHash;
use nimiq_keys::{Address, PublicKey as SchnorrPublicKey};

/// A collection of receipts for inherents/transactions. This is necessary to be able to revert
/// those inherents/transactions.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct SlashReceipt {
    pub newly_parked: bool,
    pub newly_disabled: bool,
    pub newly_lost_rewards: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct UpdateValidatorReceipt {
    pub old_signing_key: SchnorrPublicKey,
    pub old_voting_key: BlsPublicKey,
    pub old_reward_address: Address,
    pub old_signal_data: Option<Blake2bHash>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct InactivateValidatorReceipt {
    pub was_parked: bool,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct ReactivateValidatorReceipt {
    pub was_inactive_since: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct UnparkValidatorReceipt {
    #[beserial(len_type(u16))]
    pub current_disabled_slots: Option<BTreeSet<u16>>,
    #[beserial(len_type(u16))]
    pub previous_disabled_slots: Option<BTreeSet<u16>>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct DeleteValidatorReceipt {
    pub signing_key: SchnorrPublicKey,
    pub voting_key: BlsPublicKey,
    pub reward_address: Address,
    pub signal_data: Option<Blake2bHash>,
    pub inactive_since: u32,
    #[beserial(len_type(u32))]
    pub stakers: Vec<Address>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct StakerReceipt {
    pub delegation: Option<Address>,
}
