use beserial::{Deserialize, Serialize};
use nimiq_primitives::account::AccountType;
use nimiq_primitives::{account::AccountError, coin::Coin};
use nimiq_transaction::Transaction;

use crate::data_store::{DataStoreRead, DataStoreWrite};
use crate::inherent::Inherent;
use crate::interaction_traits::{AccountPruningInteraction, AccountTransactionInteraction};
use crate::{Account, AccountError, AccountInherentInteraction, AccountReceipt};

#[derive(Clone, PartialEq, PartialOrd, Eq, Ord, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "serde-derive", derive(serde::Serialize, serde::Deserialize))]
pub struct BasicAccount {
    pub balance: Coin,
}

impl AccountTransactionInteraction for BasicAccount {
    fn create_new_contract(
        _transaction: &Transaction,
        _initial_balance: Coin,
        _block_time: u64,
        _data_store: DataStoreWrite,
    ) -> Result<Account, AccountError> {
        Err(AccountError::InvalidForRecipient)
    }

    fn revert_new_contract(
        self,
        _transaction: &Transaction,
        _block_time: u64,
        _data_store: DataStoreWrite,
    ) -> Result<Account, AccountError> {
        Err(AccountError::InvalidForRecipient)
    }

    fn commit_incoming_transaction(
        &mut self,
        transaction: &Transaction,
        _block_time: u64,
        _data_store: DataStoreWrite,
    ) -> Result<Option<AccountReceipt>, AccountError> {
        Account::balance_add_assign(&mut self.balance, transaction.value)?;
        Ok(None)
    }

    fn revert_incoming_transaction(
        &mut self,
        transaction: &Transaction,
        _block_time: u64,
        _receipt: Option<&AccountReceipt>,
        _data_store: DataStoreWrite,
    ) -> Result<(), AccountError> {
        Account::balance_sub_assign(&mut self.balance, transaction.value)?;
        Ok(())
    }

    fn commit_outgoing_transaction(
        &mut self,
        transaction: &Transaction,
        _block_time: u64,
        _data_store: DataStoreWrite,
    ) -> Result<Option<AccountReceipt>, AccountError> {
        Account::balance_sub_assign(&mut self.balance, transaction.total_value())?;
        Ok(None)
    }

    fn revert_outgoing_transaction(
        &mut self,
        transaction: &Transaction,
        _block_time: u64,
        _receipt: Option<&AccountReceipt>,
        _data_store: DataStoreWrite,
    ) -> Result<(), AccountError> {
        Account::balance_add_assign(&mut self.balance, transaction.total_value())?;
        Ok(())
    }

    fn commit_failed_transaction(
        &mut self,
        transaction: &Transaction,
        _block_time: u64,
        _data_store: DataStoreWrite,
    ) -> Result<Option<AccountReceipt>, AccountError> {
        Account::balance_sub_assign(&mut self.balance, transaction.fee)?;
        Ok(None)
    }

    fn revert_failed_transaction(
        &mut self,
        transaction: &Transaction,
        _block_time: u64,
        _receipt: Option<&AccountReceipt>,
        _data_store: DataStoreWrite,
    ) -> Result<(), AccountError> {
        Account::balance_add_assign(&mut self.balance, transaction.fee)?;
        Ok(())
    }

    fn has_sufficient_balance(
        &self,
        transaction: &Transaction,
        reserved_balance: Coin,
        _block_time: u64,
        _data_store: DataStoreRead,
    ) -> Result<bool, AccountError> {
        let needed = reserved_balance
            .checked_add(transaction.total_value())
            .ok_or(AccountError::InvalidCoinValue)?;
        Ok(self.balance >= needed)
    }
}

impl AccountInherentInteraction for BasicAccount {
    fn commit_inherent(
        &mut self,
        inherent: &Inherent,
        _block_time: u64,
        _data_store: DataStoreWrite,
    ) -> Result<Option<AccountReceipt>, AccountError> {
        match inherent {
            Inherent::Reward { value, .. } => {
                Account::balance_add_assign(&mut self.balance, *value).map(|| None)
            }
            _ => Err(AccountError::InvalidForTarget),
        }
    }

    fn revert_inherent(
        &mut self,
        inherent: &Inherent,
        _block_time: u64,
        _receipt: Option<&AccountReceipt>,
        _data_store: DataStoreWrite,
    ) -> Result<(), AccountError> {
        match inherent {
            Inherent::Reward { value, .. } => {
                Account::balance_sub_assign(&mut self.balance, *value)
            }
            _ => Err(AccountError::InvalidForTarget),
        }
    }
}

impl AccountPruningInteraction for BasicAccount {
    fn can_be_pruned(&self) -> bool {
        self.balance.is_zero()
    }

    fn prune(self, _data_store: DataStoreRead) -> Result<Option<AccountReceipt>, AccountError> {
        Ok(None)
    }

    fn restore(
        _ty: AccountType,
        _pruned_account: Option<&AccountReceipt>,
        _data_store: DataStoreWrite,
    ) -> Result<Account, AccountError> {
        Ok(Account::Basic(BasicAccount::default()))
    }
}
