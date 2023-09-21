#[cfg(feature = "interaction-traits")]
use nimiq_primitives::account::{AccountError, AccountType};
use nimiq_primitives::coin::Coin;
#[cfg(feature = "interaction-traits")]
use nimiq_transaction::{inherent::Inherent, Transaction};
use serde::{Deserialize, Serialize};

#[cfg(feature = "interaction-traits")]
use crate::{
    data_store::{DataStoreRead, DataStoreWrite},
    interaction_traits::{
        AccountInherentInteraction, AccountPruningInteraction, AccountTransactionInteraction,
        BlockState,
    },
    reserved_balance::ReservedBalance,
    Account, AccountReceipt, InherentLogger, Log, TransactionLog,
};

#[derive(Clone, PartialEq, PartialOrd, Eq, Ord, Debug, Default, Serialize, Deserialize)]
pub struct BasicAccount {
    pub balance: Coin,
}

#[cfg(feature = "interaction-traits")]
impl AccountTransactionInteraction for BasicAccount {
    fn create_new_contract(
        _transaction: &Transaction,
        _initial_balance: Coin,
        _block_state: &BlockState,
        _data_store: DataStoreWrite,
        _tx_logger: &mut TransactionLog,
    ) -> Result<Account, AccountError> {
        Err(AccountError::InvalidForRecipient)
    }

    fn revert_new_contract(
        &mut self,
        _transaction: &Transaction,
        _block_state: &BlockState,
        _data_store: DataStoreWrite,
        _tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        Err(AccountError::InvalidForRecipient)
    }

    fn commit_incoming_transaction(
        &mut self,
        transaction: &Transaction,
        _block_state: &BlockState,
        _data_store: DataStoreWrite,
        _tx_logger: &mut TransactionLog,
    ) -> Result<Option<AccountReceipt>, AccountError> {
        self.balance += transaction.value;
        Ok(None)
    }

    fn revert_incoming_transaction(
        &mut self,
        transaction: &Transaction,
        _block_state: &BlockState,
        _receipt: Option<AccountReceipt>,
        _data_store: DataStoreWrite,
        _tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        self.balance -= transaction.value;
        Ok(())
    }

    fn commit_outgoing_transaction(
        &mut self,
        transaction: &Transaction,
        _block_state: &BlockState,
        _data_store: DataStoreWrite,
        tx_logger: &mut TransactionLog,
    ) -> Result<Option<AccountReceipt>, AccountError> {
        self.balance.safe_sub_assign(transaction.total_value())?;

        tx_logger.push_log(Log::pay_fee_log(transaction));
        tx_logger.push_log(Log::transfer_log(transaction));

        Ok(None)
    }

    fn revert_outgoing_transaction(
        &mut self,
        transaction: &Transaction,
        _block_state: &BlockState,
        _receipt: Option<AccountReceipt>,
        _data_store: DataStoreWrite,
        tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        self.balance += transaction.total_value();

        tx_logger.push_log(Log::transfer_log(transaction));
        tx_logger.push_log(Log::pay_fee_log(transaction));

        Ok(())
    }

    fn commit_failed_transaction(
        &mut self,
        transaction: &Transaction,
        _block_state: &BlockState,
        _data_store: DataStoreWrite,
        tx_logger: &mut TransactionLog,
    ) -> Result<Option<AccountReceipt>, AccountError> {
        self.balance.safe_sub_assign(transaction.fee)?;

        tx_logger.push_log(Log::pay_fee_log(transaction));

        Ok(None)
    }

    fn revert_failed_transaction(
        &mut self,
        transaction: &Transaction,
        _block_state: &BlockState,
        _receipt: Option<AccountReceipt>,
        _data_store: DataStoreWrite,
        tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        self.balance += transaction.fee;

        tx_logger.push_log(Log::pay_fee_log(transaction));

        Ok(())
    }

    fn reserve_balance(
        &self,
        transaction: &Transaction,
        reserved_balance: &mut ReservedBalance,
        _block_state: &BlockState,
        _data_store: DataStoreRead,
    ) -> Result<(), AccountError> {
        reserved_balance.reserve(self.balance, transaction.total_value())
    }

    fn release_balance(
        &self,
        transaction: &Transaction,
        reserved_balance: &mut ReservedBalance,
        _data_store: DataStoreRead,
    ) -> Result<(), AccountError> {
        reserved_balance.release(transaction.total_value());
        Ok(())
    }
}

#[cfg(feature = "interaction-traits")]
impl AccountInherentInteraction for BasicAccount {
    fn commit_inherent(
        &mut self,
        inherent: &Inherent,
        _block_state: &BlockState,
        _data_store: DataStoreWrite,
        inherent_logger: &mut InherentLogger,
    ) -> Result<Option<AccountReceipt>, AccountError> {
        match inherent {
            Inherent::Reward { value, target } => {
                self.balance += *value;

                inherent_logger.push_log(Log::PayoutReward {
                    to: target.clone(),
                    value: *value,
                });

                Ok(None)
            }
            _ => Err(AccountError::InvalidForTarget),
        }
    }

    fn revert_inherent(
        &mut self,
        inherent: &Inherent,
        _block_state: &BlockState,
        _receipt: Option<AccountReceipt>,
        _data_store: DataStoreWrite,
        inherent_logger: &mut InherentLogger,
    ) -> Result<(), AccountError> {
        match inherent {
            Inherent::Reward { value, target } => {
                self.balance -= *value;

                inherent_logger.push_log(Log::PayoutReward {
                    to: target.clone(),
                    value: *value,
                });

                Ok(())
            }
            _ => Err(AccountError::InvalidForTarget),
        }
    }
}

#[cfg(feature = "interaction-traits")]
impl AccountPruningInteraction for BasicAccount {
    fn can_be_pruned(&self) -> bool {
        self.balance.is_zero()
    }

    fn prune(self, _data_store: DataStoreRead) -> Option<AccountReceipt> {
        None
    }

    fn restore(
        _ty: AccountType,
        _pruned_account: Option<&AccountReceipt>,
        _data_store: DataStoreWrite,
    ) -> Result<Account, AccountError> {
        Ok(Account::Basic(BasicAccount::default()))
    }
}
