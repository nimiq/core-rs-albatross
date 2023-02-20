#[macro_use]
extern crate log;

use beserial::Deserialize;
use nimiq_database::WriteTransaction;
use nimiq_primitives::key_nibbles::KeyNibbles;
use nimiq_trie::trie::IncompleteTrie;

pub use crate::account::{
    basic_account::BasicAccount, htlc_contract::HashedTimeLockedContract,
    staking_contract::StakingContract, vesting_contract::VestingContract, Account,
};
pub use crate::accounts::{Accounts, AccountsTrie};
pub use crate::accounts_list::AccountsList;
pub use crate::data_store::DataStore;
pub use crate::inherent::Inherent;
pub use crate::interaction_traits::*;
pub use crate::receipts::*;
pub use crate::reserved_balance::ReservedBalance;

mod account;
mod accounts;
mod accounts_list;
mod data_store;
mod inherent;
mod interaction_traits;
mod logs;
mod receipts;
mod reserved_balance;

#[macro_export]
macro_rules! complete {
    ($account: expr) => {
        match $account {
            Ok(account) => account,
            Err(nimiq_trie::trie::IncompleteTrie) => {
                return Ok($crate::logs::MissingInfo::missing());
            }
        }
    };
}

/// TODO: Function used to make the partial trees work until the accounts rewrite.
pub(crate) fn get_or_update_account<T: Deserialize>(
    accounts_tree: &AccountsTrie,
    txn: &mut WriteTransaction,
    key: &KeyNibbles,
) -> Result<Option<T>, IncompleteTrie> {
    match accounts_tree.get::<T>(txn, key) {
        Err(IncompleteTrie) => {
            accounts_tree.update_within_missing_part(txn, key).unwrap();
            Err(IncompleteTrie)
        }
        account => account,
    }
}
