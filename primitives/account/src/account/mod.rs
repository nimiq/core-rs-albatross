use beserial::{Deserialize, Serialize};
use nimiq_primitives::account::AccountType;
use nimiq_primitives::coin::Coin;
use nimiq_transaction::{Transaction, TransactionFlags};

use crate::data_store::{DataStoreRead, DataStoreWrite};
use crate::interaction_traits::{
    AccountInherentInteraction, AccountPruningInteraction, AccountTransactionInteraction,
};
use crate::{
    AccountError, AccountReceipt, BasicAccount, HashedTimeLockedContract, Inherent,
    StakingContract, VestingContract,
};

pub mod basic_account;
pub mod htlc_contract;
pub mod staking_contract;
pub mod vesting_contract;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "serde-derive", derive(serde::Serialize, serde::Deserialize))]
#[repr(u8)]
pub enum Account {
    Basic(BasicAccount),
    Vesting(VestingContract),
    HTLC(HashedTimeLockedContract),
    Staking(StakingContract),
}

impl Account {
    pub fn account_type(&self) -> AccountType {
        match *self {
            Account::Basic(_) => AccountType::Basic,
            Account::Vesting(_) => AccountType::Vesting,
            Account::HTLC(_) => AccountType::HTLC,
            Account::Staking(_) => AccountType::Staking,
        }
    }

    pub fn balance(&self) -> Coin {
        match *self {
            Account::Basic(ref account) => account.balance,
            Account::Vesting(ref account) => account.balance,
            Account::HTLC(ref account) => account.balance,
            Account::Staking(ref account) => account.balance,
        }
    }

    pub(crate) fn default_with_balance(balance: Coin) -> Self {
        Account::Basic(BasicAccount { balance })
    }

    pub(crate) fn balance_add(balance: Coin, value: Coin) -> Result<Coin, AccountError> {
        balance
            .checked_add(value)
            .ok_or_else(|| AccountError::InvalidCoinValue)
    }

    pub(crate) fn balance_add_assign(balance: &mut Coin, value: Coin) -> Result<(), AccountError> {
        *balance = Self::balance_add(*balance, value)?;
        Ok(())
    }

    pub(crate) fn balance_sub(balance: Coin, value: Coin) -> Result<Coin, AccountError> {
        balance
            .checked_sub(value)
            .ok_or_else(|| AccountError::InsufficientFunds {
                balance: *balance,
                needed: value,
            })
    }

    pub(crate) fn balance_sub_assign(balance: &mut Coin, value: Coin) -> Result<(), AccountError> {
        *balance = Self::balance_sub(*balance, value)?;
        Ok(())
    }
}

impl Default for Account {
    fn default() -> Self {
        Account::Basic(BasicAccount::default())
    }
}

impl AccountTransactionInteraction for Account {
    fn create_new_contract(
        transaction: &Transaction,
        initial_balance: Coin,
        block_time: u64,
        data_store: DataStoreWrite,
    ) -> Result<Account, AccountError> {
        assert!(transaction
            .flags
            .contains(TransactionFlags::CONTRACT_CREATION));
        todo!()
    }

    fn revert_new_contract(
        self,
        transaction: &Transaction,
        block_time: u64,
        data_store: DataStoreWrite,
    ) -> Result<Account, AccountError> {
        todo!()
    }

    fn commit_incoming_transaction(
        &mut self,
        transaction: &Transaction,
        block_time: u64,
        data_store: DataStoreWrite,
    ) -> Result<Option<AccountReceipt>, AccountError> {
        todo!()
    }

    fn revert_incoming_transaction(
        &mut self,
        transaction: &Transaction,
        block_time: u64,
        receipt: Option<&AccountReceipt>,
        data_store: DataStoreWrite,
    ) -> Result<(), AccountError> {
        todo!()
    }

    fn commit_outgoing_transaction(
        &mut self,
        transaction: &Transaction,
        block_time: u64,
        data_store: DataStoreWrite,
    ) -> Result<Option<AccountReceipt>, AccountError> {
        todo!()
    }

    fn revert_outgoing_transaction(
        &mut self,
        transaction: &Transaction,
        block_time: u64,
        receipt: Option<&AccountReceipt>,
        data_store: DataStoreWrite,
    ) -> Result<(), AccountError> {
        todo!()
    }

    fn commit_failed_transaction(
        &mut self,
        transaction: &Transaction,
        block_time: u64,
        data_store: DataStoreWrite,
    ) -> Result<Option<AccountReceipt>, AccountError> {
        todo!()
    }

    fn revert_failed_transaction(
        &mut self,
        transaction: &Transaction,
        block_time: u64,
        receipt: Option<&AccountReceipt>,
        data_store: DataStoreWrite,
    ) -> Result<(), AccountError> {
        todo!()
    }

    fn has_sufficient_balance(
        &self,
        transaction: &Transaction,
        reserved_balance: Coin,
        block_time: u64,
        data_store: DataStoreRead,
    ) -> Result<bool, AccountError> {
        todo!()
    }

    // fn create(
    //     accounts_tree: &AccountsTrie,
    //     db_txn: &mut WriteTransaction,
    //     transaction: &Transaction,
    //     block_height: u32,
    //     block_time: u64,
    // ) -> Result<AccountInfo, AccountError> {
    //     match transaction.recipient_type {
    //         AccountType::Vesting => VestingContract::create(
    //             accounts_tree,
    //             db_txn,
    //             transaction,
    //             block_height,
    //             block_time,
    //         ),
    //         AccountType::HTLC => HashedTimeLockedContract::create(
    //             accounts_tree,
    //             db_txn,
    //             transaction,
    //             block_height,
    //             block_time,
    //         ),
    //         _ => Err(AccountError::InvalidForRecipient),
    //     }
    // }
    //
    // fn commit_incoming_transaction(
    //     accounts_tree: &AccountsTrie,
    //     db_txn: &mut WriteTransaction,
    //     transaction: &Transaction,
    //     block_height: u32,
    //     block_time: u64,
    // ) -> Result<AccountInfo, AccountError> {
    //     match transaction.recipient_type {
    //         AccountType::Basic => BasicAccount::commit_incoming_transaction(
    //             accounts_tree,
    //             db_txn,
    //             transaction,
    //             block_height,
    //             block_time,
    //         ),
    //         AccountType::Vesting => VestingContract::commit_incoming_transaction(
    //             accounts_tree,
    //             db_txn,
    //             transaction,
    //             block_height,
    //             block_time,
    //         ),
    //         AccountType::HTLC => HashedTimeLockedContract::commit_incoming_transaction(
    //             accounts_tree,
    //             db_txn,
    //             transaction,
    //             block_height,
    //             block_time,
    //         ),
    //         AccountType::Staking => StakingContract::commit_incoming_transaction(
    //             accounts_tree,
    //             db_txn,
    //             transaction,
    //             block_height,
    //             block_time,
    //         ),
    //     }
    // }
    //
    // fn revert_incoming_transaction(
    //     accounts_tree: &AccountsTrie,
    //     db_txn: &mut WriteTransaction,
    //     transaction: &Transaction,
    //     block_height: u32,
    //     block_time: u64,
    //     receipt: Option<&Vec<u8>>,
    // ) -> Result<Vec<Log>, AccountError> {
    //     match transaction.recipient_type {
    //         AccountType::Basic => BasicAccount::revert_incoming_transaction(
    //             accounts_tree,
    //             db_txn,
    //             transaction,
    //             block_height,
    //             block_time,
    //             receipt,
    //         ),
    //         AccountType::Vesting => VestingContract::revert_incoming_transaction(
    //             accounts_tree,
    //             db_txn,
    //             transaction,
    //             block_height,
    //             block_time,
    //             receipt,
    //         ),
    //         AccountType::HTLC => HashedTimeLockedContract::revert_incoming_transaction(
    //             accounts_tree,
    //             db_txn,
    //             transaction,
    //             block_height,
    //             block_time,
    //             receipt,
    //         ),
    //         AccountType::Staking => StakingContract::revert_incoming_transaction(
    //             accounts_tree,
    //             db_txn,
    //             transaction,
    //             block_height,
    //             block_time,
    //             receipt,
    //         ),
    //     }
    // }
    //
    // fn commit_outgoing_transaction(
    //     accounts_tree: &AccountsTrie,
    //     db_txn: &mut WriteTransaction,
    //     transaction: &Transaction,
    //     block_height: u32,
    //     block_time: u64,
    // ) -> Result<AccountInfo, AccountError> {
    //     match transaction.sender_type {
    //         AccountType::Basic => BasicAccount::commit_outgoing_transaction(
    //             accounts_tree,
    //             db_txn,
    //             transaction,
    //             block_height,
    //             block_time,
    //         ),
    //         AccountType::Vesting => VestingContract::commit_outgoing_transaction(
    //             accounts_tree,
    //             db_txn,
    //             transaction,
    //             block_height,
    //             block_time,
    //         ),
    //         AccountType::HTLC => HashedTimeLockedContract::commit_outgoing_transaction(
    //             accounts_tree,
    //             db_txn,
    //             transaction,
    //             block_height,
    //             block_time,
    //         ),
    //         AccountType::Staking => StakingContract::commit_outgoing_transaction(
    //             accounts_tree,
    //             db_txn,
    //             transaction,
    //             block_height,
    //             block_time,
    //         ),
    //     }
    // }
    //
    // fn revert_outgoing_transaction(
    //     accounts_tree: &AccountsTrie,
    //     db_txn: &mut WriteTransaction,
    //     transaction: &Transaction,
    //     block_height: u32,
    //     block_time: u64,
    //     receipt: Option<&Vec<u8>>,
    // ) -> Result<Vec<Log>, AccountError> {
    //     match transaction.sender_type {
    //         AccountType::Basic => BasicAccount::revert_outgoing_transaction(
    //             accounts_tree,
    //             db_txn,
    //             transaction,
    //             block_height,
    //             block_time,
    //             receipt,
    //         ),
    //         AccountType::Vesting => VestingContract::revert_outgoing_transaction(
    //             accounts_tree,
    //             db_txn,
    //             transaction,
    //             block_height,
    //             block_time,
    //             receipt,
    //         ),
    //         AccountType::HTLC => HashedTimeLockedContract::revert_outgoing_transaction(
    //             accounts_tree,
    //             db_txn,
    //             transaction,
    //             block_height,
    //             block_time,
    //             receipt,
    //         ),
    //         AccountType::Staking => StakingContract::revert_outgoing_transaction(
    //             accounts_tree,
    //             db_txn,
    //             transaction,
    //             block_height,
    //             block_time,
    //             receipt,
    //         ),
    //     }
    // }
    // fn commit_failed_transaction(
    //     accounts_tree: &AccountsTrie,
    //     db_txn: &mut WriteTransaction,
    //     transaction: &Transaction,
    //     block_height: u32,
    // ) -> Result<AccountInfo, AccountError> {
    //     // Committing a failed transaction is based on the sender type.
    //     // The fee needs to be paid from the sender account.
    //     match transaction.sender_type {
    //         AccountType::Basic => BasicAccount::commit_failed_transaction(
    //             accounts_tree,
    //             db_txn,
    //             transaction,
    //             block_height,
    //         ),
    //         AccountType::Vesting => VestingContract::commit_failed_transaction(
    //             accounts_tree,
    //             db_txn,
    //             transaction,
    //             block_height,
    //         ),
    //         AccountType::HTLC => HashedTimeLockedContract::commit_failed_transaction(
    //             accounts_tree,
    //             db_txn,
    //             transaction,
    //             block_height,
    //         ),
    //         AccountType::Staking => StakingContract::commit_failed_transaction(
    //             accounts_tree,
    //             db_txn,
    //             transaction,
    //             block_height,
    //         ),
    //     }
    // }
    // fn revert_failed_transaction(
    //     accounts_tree: &AccountsTrie,
    //     db_txn: &mut WriteTransaction,
    //     transaction: &Transaction,
    //
    //     receipt: Option<&Vec<u8>>,
    // ) -> Result<Vec<Log>, AccountError> {
    //     match transaction.sender_type {
    //         AccountType::Basic => {
    //             BasicAccount::revert_failed_transaction(accounts_tree, db_txn, transaction, receipt)
    //         }
    //         AccountType::Vesting => VestingContract::revert_failed_transaction(
    //             accounts_tree,
    //             db_txn,
    //             transaction,
    //             receipt,
    //         ),
    //         AccountType::HTLC => HashedTimeLockedContract::revert_failed_transaction(
    //             accounts_tree,
    //             db_txn,
    //             transaction,
    //             receipt,
    //         ),
    //         AccountType::Staking => StakingContract::revert_failed_transaction(
    //             accounts_tree,
    //             db_txn,
    //             transaction,
    //             receipt,
    //         ),
    //     }
    // }
    //
    // fn can_pay_fee(
    //     &self,
    //     transaction: &Transaction,
    //     current_balance: Coin,
    //     block_time: u64,
    // ) -> bool {
    //     match &self {
    //         Account::Basic(account) => {
    //             BasicAccount::can_pay_fee(account, transaction, current_balance, block_time)
    //         }
    //         Account::Vesting(account) => {
    //             VestingContract::can_pay_fee(account, transaction, current_balance, block_time)
    //         }
    //         Account::HTLC(account) => HashedTimeLockedContract::can_pay_fee(
    //             account,
    //             transaction,
    //             current_balance,
    //             block_time,
    //         ),
    //         Account::Staking(account) => {
    //             StakingContract::can_pay_fee(account, transaction, current_balance, block_time)
    //         }
    //         _ => false,
    //     }
    // }
    //
    // fn delete(
    //     accounts_tree: &AccountsTrie,
    //     db_txn: &mut WriteTransaction,
    //     transaction: &Transaction,
    // ) -> Result<Vec<Log>, AccountError> {
    //     match transaction.recipient_type {
    //         AccountType::Vesting => VestingContract::delete(accounts_tree, db_txn, transaction),
    //         AccountType::HTLC => {
    //             HashedTimeLockedContract::delete(accounts_tree, db_txn, transaction)
    //         }
    //         _ => Err(AccountError::InvalidForRecipient),
    //     }
    // }
}

impl AccountInherentInteraction for Account {
    fn commit_inherent(
        &mut self,
        inherent: &Inherent,
        block_time: u64,
        data_store: DataStoreWrite,
    ) -> Result<Option<AccountReceipt>, AccountError> {
        todo!()
    }

    fn revert_inherent(
        &mut self,
        inherent: &Inherent,
        block_time: u64,
        receipt: Option<&AccountReceipt>,
        data_store: DataStoreWrite,
    ) -> Result<(), AccountError> {
        todo!()
    }

    // fn commit_inherent(
    //     accounts_tree: &AccountsTrie,
    //     db_txn: &mut WriteTransaction,
    //     inherent: &Inherent,
    //     block_height: u32,
    //     block_time: u64,
    // ) -> Result<AccountInfo, AccountError> {
    //     // If the inherent target is the staking contract then we forward it to the staking contract
    //     // right here.
    //     if STAKING_CONTRACT_ADDRESS == inherent.target {
    //         return StakingContract::commit_inherent(
    //             accounts_tree,
    //             db_txn,
    //             inherent,
    //             block_height,
    //             block_time,
    //         );
    //     }
    //
    //     // Otherwise, we need to check if the target address belongs to a basic account (or a
    //     // non-existent account).
    //     let key = KeyNibbles::from(&inherent.target);
    //
    //     let account_type = match accounts_tree.get::<Account>(db_txn, &key) {
    //         Some(x) => x.account_type(),
    //         None => AccountType::Basic,
    //     };
    //
    //     if account_type == AccountType::Basic {
    //         BasicAccount::commit_inherent(accounts_tree, db_txn, inherent, block_height, block_time)
    //     } else {
    //         Err(AccountError::InvalidInherent)
    //     }
    // }
    //
    // fn revert_inherent(
    //     accounts_tree: &AccountsTrie,
    //     db_txn: &mut WriteTransaction,
    //     inherent: &Inherent,
    //     block_height: u32,
    //     block_time: u64,
    //     receipt: Option<&Vec<u8>>,
    // ) -> Result<Vec<Log>, AccountError> {
    //     // If the inherent target is the staking contract then we forward it to the staking contract
    //     // right here.
    //     if STAKING_CONTRACT_ADDRESS == inherent.target {
    //         return StakingContract::revert_inherent(
    //             accounts_tree,
    //             db_txn,
    //             inherent,
    //             block_height,
    //             block_time,
    //             receipt,
    //         );
    //     }
    //
    //     // Otherwise, we need to check if the target address belongs to a basic account (or a
    //     // non-existent account).
    //     let key = KeyNibbles::from(&inherent.target);
    //
    //     let account_type = match accounts_tree.get::<Account>(db_txn, &key) {
    //         Some(x) => x.account_type(),
    //         None => AccountType::Basic,
    //     };
    //
    //     if account_type == AccountType::Basic {
    //         BasicAccount::revert_inherent(
    //             accounts_tree,
    //             db_txn,
    //             inherent,
    //             block_height,
    //             block_time,
    //             receipt,
    //         )
    //     } else {
    //         Err(AccountError::InvalidInherent)
    //     }
    // }
}

impl AccountPruningInteraction for Account {
    fn can_be_pruned(&self) -> bool {
        todo!()
    }

    fn prune(self, data_store: DataStoreRead) -> Result<Option<AccountReceipt>, AccountError> {
        todo!()
    }

    fn restore(
        ty: AccountType,
        pruned_account: Option<&AccountReceipt>,
        data_store: DataStoreWrite,
    ) -> Result<Account, AccountError> {
        todo!()
    }
}
