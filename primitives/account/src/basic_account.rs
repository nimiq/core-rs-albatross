use beserial::{Deserialize, Serialize};
use primitives::coin::Coin;
use transaction::Transaction;

use crate::Account;
use crate::AccountError;
use crate::AccountTransactionInteraction;
use crate::AccountType;

#[derive(Clone, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
pub struct BasicAccount {
    pub balance: Coin
}

impl AccountTransactionInteraction for BasicAccount {
    fn new_contract(_account_type: AccountType, _balance: Coin, _transaction: &Transaction, _block_height: u32) -> Result<Self, AccountError> {
        Err(AccountError::InvalidForRecipient)
    }

    fn create(_balance: Coin, _transaction: &Transaction, _block_height: u32) -> Result<Self, AccountError> {
        Err(AccountError::InvalidForRecipient)
    }

    fn check_incoming_transaction(&self, transaction: &Transaction, _block_height: u32) -> Result<(), AccountError> {
        Account::balance_add(self.balance, transaction.value)?;
        Ok(())
    }

    fn commit_incoming_transaction(&mut self, transaction: &Transaction, _block_height: u32) -> Result<Option<Vec<u8>>, AccountError> {
        self.balance = Account::balance_add(self.balance, transaction.value)?;
        Ok(None)
    }

    fn revert_incoming_transaction(&mut self, transaction: &Transaction, _block_height: u32, receipt: Option<&Vec<u8>>) -> Result<(), AccountError> {
        if receipt.is_some() {
            return Err(AccountError::InvalidReceipt);
        }

        self.balance = Account::balance_sub(self.balance, transaction.value)?;
        Ok(())
    }

    fn check_outgoing_transaction(&self, transaction: &Transaction, _block_height: u32) -> Result<(), AccountError> {
        Account::balance_sub(self.balance, transaction.total_value().ok_or(AccountError::InvalidCoinValue)?)?;
        Ok(())
    }

    fn commit_outgoing_transaction(&mut self, transaction: &Transaction, _block_height: u32) -> Result<Option<Vec<u8>>, AccountError> {
        self.balance = Account::balance_sub(self.balance, transaction.total_value().ok_or(AccountError::InvalidCoinValue)?)?;
        Ok(None)
    }

    fn revert_outgoing_transaction(&mut self, transaction: &Transaction, _block_height: u32, receipt: Option<&Vec<u8>>) -> Result<(), AccountError> {
        if receipt.is_some() {
            return Err(AccountError::InvalidReceipt);
        }

        self.balance = Account::balance_add(self.balance, transaction.total_value().ok_or(AccountError::InvalidCoinValue)?)?;
        Ok(())
    }
}
