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

    fn with_incoming_transaction(&self, transaction: &Transaction, _block_height: u32) -> Result<Self, AccountError> {
        let balance: Coin = Account::balance_add(self.balance, transaction.value)?;
        return Ok(BasicAccount { balance });
    }

    fn without_incoming_transaction(&self, transaction: &Transaction, _block_height: u32) -> Result<Self, AccountError> {
        let balance: Coin = Account::balance_sub(self.balance, transaction.value)?;
        return Ok(BasicAccount { balance });
    }

    fn with_outgoing_transaction(&self, transaction: &Transaction, _block_height: u32) -> Result<Self, AccountError> {
        let balance: Coin = Account::balance_sub(self.balance, transaction.value + transaction.fee)?;
        return Ok(BasicAccount { balance });
    }

    fn without_outgoing_transaction(&self, transaction: &Transaction, _block_height: u32) -> Result<Self, AccountError> {
        let balance: Coin = Account::balance_add(self.balance, transaction.value + transaction.fee)?;
        return Ok(BasicAccount { balance });
    }
}
