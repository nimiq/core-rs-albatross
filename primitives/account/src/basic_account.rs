use beserial::{Deserialize, Serialize};
use nimiq_database::WriteTransaction;
use nimiq_primitives::account::AccountType;
use nimiq_primitives::coin::Coin;
use nimiq_transaction::Transaction;
use nimiq_trie::key_nibbles::KeyNibbles;

use crate::inherent::{Inherent, InherentType};
use crate::interaction_traits::{AccountInherentInteraction, AccountTransactionInteraction};
use crate::{Account, AccountError, AccountsTrie};

#[derive(Clone, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "serde-derive", derive(serde::Serialize, serde::Deserialize))]
pub struct BasicAccount {
    pub balance: Coin,
}

impl AccountTransactionInteraction for BasicAccount {
    fn create(
        _accounts_tree: &AccountsTrie,
        _db_txn: &mut WriteTransaction,
        _transaction: &Transaction,
        _block_height: u32,
        _block_time: u64,
    ) -> Result<(), AccountError> {
        Err(AccountError::InvalidForRecipient)
    }

    fn commit_incoming_transaction(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        transaction: &Transaction,
        _block_height: u32,
        _block_time: u64,
    ) -> Result<Option<Vec<u8>>, AccountError> {
        let key = KeyNibbles::from(&transaction.recipient);

        let leaf = accounts_tree.get(db_txn, &key);

        // Implicitly also checks that the address is in fact from a basic account.
        let current_balance = match leaf {
            Some(Account::Basic(account)) => account.balance,
            None => Coin::ZERO,
            _ => {
                return Err(AccountError::TypeMismatch {
                    expected: AccountType::Basic,
                    got: leaf.unwrap().account_type(),
                })
            }
        };

        let new_balance = Account::balance_add(current_balance, transaction.value)?;

        accounts_tree.put(
            db_txn,
            &key,
            Account::Basic(BasicAccount {
                balance: new_balance,
            }),
        );

        Ok(None)
    }

    fn revert_incoming_transaction(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        transaction: &Transaction,
        _block_height: u32,
        _block_time: u64,
        receipt: Option<&Vec<u8>>,
    ) -> Result<(), AccountError> {
        if receipt.is_some() {
            return Err(AccountError::InvalidReceipt);
        }

        let key = KeyNibbles::from(&transaction.recipient);

        let account = accounts_tree
            .get(db_txn, &key)
            .ok_or(AccountError::NonExistentAddress {
                address: transaction.recipient.clone(),
            })?;

        let new_balance = Account::balance_sub(account.balance(), transaction.value)?;

        if new_balance.is_zero() {
            accounts_tree.remove(db_txn, &key);
        } else {
            accounts_tree.put(
                db_txn,
                &key,
                Account::Basic(BasicAccount {
                    balance: new_balance,
                }),
            );
        }

        Ok(())
    }

    fn commit_outgoing_transaction(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        transaction: &Transaction,
        _block_height: u32,
        _block_time: u64,
    ) -> Result<Option<Vec<u8>>, AccountError> {
        let key = KeyNibbles::from(&transaction.sender);

        let account = accounts_tree
            .get(db_txn, &key)
            .ok_or(AccountError::NonExistentAddress {
                address: transaction.sender.clone(),
            })?;

        if account.account_type() != AccountType::Basic {
            return Err(AccountError::TypeMismatch {
                expected: AccountType::Basic,
                got: account.account_type(),
            });
        }

        let new_balance = Account::balance_sub(account.balance(), transaction.total_value()?)?;

        if new_balance.is_zero() {
            accounts_tree.remove(db_txn, &key);
        } else {
            accounts_tree.put(
                db_txn,
                &key,
                Account::Basic(BasicAccount {
                    balance: new_balance,
                }),
            );
        }

        Ok(None)
    }

    fn revert_outgoing_transaction(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        transaction: &Transaction,
        _block_height: u32,
        _block_time: u64,
        receipt: Option<&Vec<u8>>,
    ) -> Result<(), AccountError> {
        if receipt.is_some() {
            return Err(AccountError::InvalidReceipt);
        }

        let key = KeyNibbles::from(&transaction.sender);

        let leaf = accounts_tree.get(db_txn, &key);

        let current_balance = match leaf {
            None => Coin::ZERO,
            Some(account) => account.balance(),
        };

        let new_balance = Account::balance_add(current_balance, transaction.total_value()?)?;

        accounts_tree.put(
            db_txn,
            &key,
            Account::Basic(BasicAccount {
                balance: new_balance,
            }),
        );

        Ok(())
    }
}

impl AccountInherentInteraction for BasicAccount {
    fn commit_inherent(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        inherent: &Inherent,
        _block_height: u32,
        _block_time: u64,
    ) -> Result<Option<Vec<u8>>, AccountError> {
        if inherent.ty != InherentType::Reward {
            return Err(AccountError::InvalidInherent);
        }

        let key = KeyNibbles::from(&inherent.target);

        let leaf = accounts_tree.get(db_txn, &key);

        let current_balance = match leaf {
            None => Coin::ZERO,
            Some(account) => account.balance(),
        };

        let new_balance = Account::balance_add(current_balance, inherent.value)?;

        accounts_tree.put(
            db_txn,
            &key,
            Account::Basic(BasicAccount {
                balance: new_balance,
            }),
        );

        Ok(None)
    }

    fn revert_inherent(
        accounts_tree: &AccountsTrie,
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

        let key = KeyNibbles::from(&inherent.target);

        let account = accounts_tree
            .get(db_txn, &key)
            .ok_or(AccountError::NonExistentAddress {
                address: inherent.target.clone(),
            })?;

        let new_balance = Account::balance_sub(account.balance(), inherent.value)?;

        accounts_tree.put(
            db_txn,
            &key,
            Account::Basic(BasicAccount {
                balance: new_balance,
            }),
        );

        Ok(())
    }
}
