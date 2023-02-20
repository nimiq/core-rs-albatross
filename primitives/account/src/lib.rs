#[macro_use]
extern crate log;

use beserial::Deserialize;
use nimiq_database::WriteTransaction;
use nimiq_primitives::key_nibbles::KeyNibbles;
use nimiq_trie::trie::IncompleteTrie;

pub use crate::account::Account;
pub use crate::accounts::{Accounts, AccountsTrie};
pub use crate::accounts_list::AccountsList;
pub use crate::basic_account::BasicAccount;
pub use crate::htlc_contract::*;
pub use crate::interaction_traits::*;
pub use crate::logs::*;
pub use crate::receipts::*;
pub use crate::staking_contract::*;
pub use crate::vesting_contract::*;

mod account;
mod accounts;
mod accounts_list;
mod basic_account;
mod htlc_contract;

mod interaction_traits;
mod logs;
mod receipts;
mod staking_contract;
mod vesting_contract;

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
