use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::mem;
use std::ops::Add;

use beserial::{Deserialize, Serialize};
use nimiq_collections::BitSet;
use nimiq_database::WriteTransaction;
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_primitives::account::{AccountType, ValidatorId};
use nimiq_primitives::coin::Coin;
use nimiq_primitives::policy;
use nimiq_primitives::slots::SlashedSlot;
use nimiq_transaction::account::staking_contract::{
    IncomingStakingTransactionData, OutgoingStakingTransactionProof, SelfStakingTransactionData,
};
use nimiq_transaction::Transaction;
use nimiq_trie::key_nibbles::KeyNibbles;

use crate::interaction_traits::{AccountInherentInteraction, AccountTransactionInteraction};
use crate::staking_contract::actions::staker::InactiveStakeReceipt;
use crate::staking_contract::actions::validator::{
    DropValidatorReceipt, InactiveValidatorReceipt, UnparkReceipt, UpdateValidatorReceipt,
};
use crate::staking_contract::SlashReceipt;
use crate::{Account, AccountError, AccountsTree, Inherent, InherentType, StakingContract};
use nimiq_keys::Address;

/// A receipt for slash inherents. It shows whether a given slot or validator was newly disabled,
/// lost rewards or parked by a specific slash inherent. This is necessary to be able to revert
/// slash inherents.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct SlashReceipt {
    pub newly_parked: bool,
    pub newly_disabled: bool,
    pub newly_lost_rewards: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct UnparkReceipt {
    pub current_epoch: bool,
    pub previous_epoch: bool,
    #[beserial(len_type(u16))]
    pub current_disabled_slots: Option<BTreeSet<u16>>,
    #[beserial(len_type(u16))]
    pub previous_disabled_slots: Option<BTreeSet<u16>>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct UpdateValidatorReceipt {
    pub old_reward_address: Address,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct RetirementReceipt {
    pub stake: Coin,
    pub inactive_stake_receipt: Option<InactiveStakeReceipt>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct DropValidatorReceipt {
    pub reward_address: Address,
    #[beserial(len_type(u32))]
    pub retirement_by_address: BTreeMap<Address, RetirementReceipt>,
    pub retire_time: u32,
    pub unpark_receipt: UnparkReceipt,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct InactiveValidatorReceipt {
    pub retire_time: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct InactiveStakeReceipt {
    pub retire_time: u32,
}
