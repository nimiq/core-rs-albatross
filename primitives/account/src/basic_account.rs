use beserial::{Deserialize, Serialize};
use nimiq_primitives::account::AccountType;
use nimiq_primitives::coin::Coin;
use nimiq_transaction::Transaction;

use crate::inherent::{Inherent, InherentType};
use crate::interaction_traits::{AccountInherentInteraction, AccountTransactionInteraction};
use crate::{Account, AccountError, AccountsTree};
use nimiq_database::WriteTransaction;
use nimiq_trie::key_nibbles::KeyNibbles;

#[derive(Clone, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "serde-derive", derive(serde::Serialize, serde::Deserialize))]
pub struct BasicAccount {
    pub balance: Coin,
}

impl AccountTransactionInteraction for BasicAccount {
    fn create(
        _accounts_tree: &AccountsTree,
        _db_txn: &mut WriteTransaction,
        _balance: Coin,
        _transaction: &Transaction,
        _block_height: u32,
        _block_time: u64,
    ) -> Result<(), AccountError> {
        Err(AccountError::InvalidForRecipient)
    }

    fn commit_incoming_transaction(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        transaction: &Transaction,
        _block_height: u32,
        _block_time: u64,
    ) -> Result<Option<Vec<u8>>, AccountError> {
        let key = KeyNibbles::from(transaction.recipient.clone());

        let leaf = accounts_tree.get(db_txn, &key);

        let current_balance = match leaf {
            None => Coin::zero(),
            Some(account) => account.balance(),
        };

        let new_balance = Account::balance_add(current_balance, transaction.value)?;

        accounts_tree.put(
            db_txn,
            &key,
            Account(BasicAccount {
                balance: new_balance,
            }),
        );

        Ok(None)
    }

    fn revert_incoming_transaction(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        transaction: &Transaction,
        _block_height: u32,
        _block_time: u64,
        receipt: Option<&Vec<u8>>,
    ) -> Result<(), AccountError> {
        if receipt.is_some() {
            return Err(AccountError::InvalidReceipt);
        }

        let key = KeyNibbles::from(transaction.recipient.clone());

        let account = accounts_tree
            .get(db_txn, &key)
            .ok_or(AccountError::NonExistentAddress {
                address: transaction.recipient.clone(),
            })?;

        let new_balance = Account::balance_add(account.balance(), transaction.value)?;

        accounts_tree.put(
            db_txn,
            &key,
            Account(BasicAccount {
                balance: new_balance,
            }),
        );

        Ok(())
    }

    fn commit_outgoing_transaction(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        transaction: &Transaction,
        _block_height: u32,
        _block_time: u64,
    ) -> Result<Option<Vec<u8>>, AccountError> {
        let key = KeyNibbles::from(transaction.sender.clone());

        let account = accounts_tree
            .get(db_txn, &key)
            .ok_or(AccountError::NonExistentAddress {
                address: transaction.sender.clone(),
            })?;

        let new_balance = Account::balance_sub(account.balance(), transaction.total_value()?)?;

        if new_balance.is_zero() {
            accounts_tree.remove(db_txn, &key);
        } else {
            accounts_tree.put(
                db_txn,
                &key,
                Account(BasicAccount {
                    balance: new_balance,
                }),
            );
        }

        Ok(None)
    }

    fn revert_outgoing_transaction(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        transaction: &Transaction,
        _block_height: u32,
        _block_time: u64,
        receipt: Option<&Vec<u8>>,
    ) -> Result<(), AccountError> {
        if receipt.is_some() {
            return Err(AccountError::InvalidReceipt);
        }

        let key = KeyNibbles::from(transaction.sender.clone());

        let leaf = accounts_tree.get(db_txn, &key);

        let current_balance = match leaf {
            None => Coin::zero(),
            Some(account) => account.balance(),
        };

        let new_balance = Account::balance_add(current_balance, transaction.total_value()?)?;

        accounts_tree.put(
            db_txn,
            &key,
            Account(BasicAccount {
                balance: new_balance,
            }),
        );

        Ok(())
    }
}

impl AccountInherentInteraction for BasicAccount {
    fn commit_inherent(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        inherent: &Inherent,
        _block_height: u32,
        _block_time: u64,
    ) -> Result<Option<Vec<u8>>, AccountError> {
        if inherent.ty != InherentType::Reward {
            return Err(AccountError::InvalidInherent);
        }

        let key = KeyNibbles::from(inherent.target.clone());

        let leaf = accounts_tree.get(db_txn, &key);

        let current_balance = match leaf {
            None => Coin::zero(),
            Some(account) => account.balance(),
        };

        let new_balance = Account::balance_add(current_balance, inherent.value)?;

        accounts_tree.put(
            db_txn,
            &key,
            Account(BasicAccount {
                balance: new_balance,
            }),
        );

        Ok(None)
    }

    fn revert_inherent(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        inherent: &Inherent,
        _block_height: u32,
        _block_time: u64,
        receipt: Option<&Vec<u8>>,
    ) -> Result<(), AccountError> {
        if receipt.is_some() {
            return Err(AccountError::InvalidReceipt);
        }

        if inherent.ty != InherentType::Reward {
            return Err(AccountError::InvalidInherent);
        }

        let key = KeyNibbles::from(inherent.target.clone());

        let account = accounts_tree
            .get(db_txn, &key)
            .ok_or(AccountError::NonExistentAddress {
                address: inherent.target.clone(),
            })?;

        let new_balance = Account::balance_sub(account.balance(), inherent.value)?;

        accounts_tree.put(
            db_txn,
            &key,
            Account(BasicAccount {
                balance: new_balance,
            }),
        );

        Ok(())
    }
}
