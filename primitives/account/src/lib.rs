#[cfg(feature = "accounts")]
#[macro_use]
extern crate log;

use nimiq_primitives::{account::AccountError, trie::error::MerkleRadixTrieError};
use nimiq_transaction::{inherent::Inherent, Transaction};
use thiserror::Error;

#[cfg(feature = "accounts")]
pub use crate::accounts::{Accounts, AccountsTrie};
#[cfg(feature = "interaction-traits")]
pub use crate::data_store::{DataStore, DataStoreRead, DataStoreWrite};
#[cfg(feature = "interaction-traits")]
pub use crate::interaction_traits::*;
pub use crate::{
    account::{
        basic_account::BasicAccount, htlc_contract::HashedTimeLockedContract, staking_contract::*,
        vesting_contract::VestingContract, Account,
    },
    data_store_ops::DataStoreReadOps,
    logs::*,
    receipts::*,
    reserved_balance::ReservedBalance,
};

mod account;
#[cfg(feature = "accounts")]
mod accounts;
#[cfg(feature = "interaction-traits")]
mod data_store;
mod data_store_ops;
#[cfg(feature = "interaction-traits")]
mod interaction_traits;
mod logs;
mod receipts;
mod reserved_balance;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AccountsError {
    #[error("Invalid transaction: error={0}, transaction={1:?}")]
    InvalidTransaction(AccountError, Transaction),
    #[error("Invalid inherent: error={0}, inherent={1:?}")]
    InvalidInherent(AccountError, Inherent),
    #[error("Invalid diff: {0}")]
    InvalidDiff(#[from] MerkleRadixTrieError),
}
