#[cfg(feature = "interaction-traits")]
use nimiq_primitives::account::AccountError;
use nimiq_primitives::{account::AccountType, coin::Coin};
#[cfg(feature = "interaction-traits")]
use nimiq_transaction::{inherent::Inherent, Transaction, TransactionFlags};
use serde::{Deserialize, Serialize};

use crate::account::{
    basic_account::BasicAccount, htlc_contract::HashedTimeLockedContract,
    staking_contract::StakingContract, vesting_contract::VestingContract,
};
#[cfg(feature = "interaction-traits")]
use crate::{
    data_store::{DataStoreRead, DataStoreWrite},
    interaction_traits::{
        AccountInherentInteraction, AccountPruningInteraction, AccountTransactionInteraction,
        BlockState,
    },
    reserved_balance::ReservedBalance,
    AccountReceipt, InherentLogger, TransactionLog,
};

pub mod basic_account;
pub mod htlc_contract;
pub mod staking_contract;
pub mod vesting_contract;

#[cfg(feature = "interaction-traits")]
macro_rules! gen_account_match {
    ($self: ident, $f: ident $(, $arg:expr )*) => {
        match $self {
            Account::Basic(account) => account.$f($( $arg ),*),
            Account::Vesting(account) => account.$f($( $arg ),*),
            Account::HTLC(account) => account.$f($( $arg ),*),
            Account::Staking(account) => account.$f($( $arg ),*),
        }
    };
}

#[cfg(feature = "interaction-traits")]
macro_rules! gen_account_type_match {
    ($self: expr, $f: ident $(, $arg:expr )*) => {
        match $self {
            AccountType::Basic => BasicAccount::$f($( $arg ),*),
            AccountType::Vesting => VestingContract::$f($( $arg ),*),
            AccountType::HTLC => HashedTimeLockedContract::$f($( $arg ),*),
            AccountType::Staking => StakingContract::$f($( $arg ),*),
        }
    };
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

    #[cfg(feature = "interaction-traits")]
    pub(crate) fn default_with_balance(balance: Coin) -> Self {
        Account::Basic(BasicAccount { balance })
    }
}

impl Default for Account {
    fn default() -> Self {
        Account::Basic(BasicAccount::default())
    }
}

#[cfg(feature = "interaction-traits")]
impl AccountTransactionInteraction for Account {
    fn create_new_contract(
        transaction: &Transaction,
        initial_balance: Coin,
        block_state: &BlockState,
        data_store: DataStoreWrite,
        tx_logger: &mut TransactionLog,
    ) -> Result<Account, AccountError> {
        assert!(transaction
            .flags
            .contains(TransactionFlags::CONTRACT_CREATION));
        gen_account_type_match!(
            transaction.recipient_type,
            create_new_contract,
            transaction,
            initial_balance,
            block_state,
            data_store,
            tx_logger
        )
    }

    fn revert_new_contract(
        &mut self,
        transaction: &Transaction,
        block_state: &BlockState,
        data_store: DataStoreWrite,
        tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        gen_account_match!(
            self,
            revert_new_contract,
            transaction,
            block_state,
            data_store,
            tx_logger
        )
    }

    fn commit_incoming_transaction(
        &mut self,
        transaction: &Transaction,
        block_state: &BlockState,
        data_store: DataStoreWrite,
        tx_logger: &mut TransactionLog,
    ) -> Result<Option<AccountReceipt>, AccountError> {
        gen_account_match!(
            self,
            commit_incoming_transaction,
            transaction,
            block_state,
            data_store,
            tx_logger
        )
    }

    fn revert_incoming_transaction(
        &mut self,
        transaction: &Transaction,
        block_state: &BlockState,
        receipt: Option<AccountReceipt>,
        data_store: DataStoreWrite,
        tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        gen_account_match!(
            self,
            revert_incoming_transaction,
            transaction,
            block_state,
            receipt,
            data_store,
            tx_logger
        )
    }

    fn commit_outgoing_transaction(
        &mut self,
        transaction: &Transaction,
        block_state: &BlockState,
        data_store: DataStoreWrite,
        tx_logger: &mut TransactionLog,
    ) -> Result<Option<AccountReceipt>, AccountError> {
        gen_account_match!(
            self,
            commit_outgoing_transaction,
            transaction,
            block_state,
            data_store,
            tx_logger
        )
    }

    fn revert_outgoing_transaction(
        &mut self,
        transaction: &Transaction,
        block_state: &BlockState,
        receipt: Option<AccountReceipt>,
        data_store: DataStoreWrite,
        tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        gen_account_match!(
            self,
            revert_outgoing_transaction,
            transaction,
            block_state,
            receipt,
            data_store,
            tx_logger
        )
    }

    fn commit_failed_transaction(
        &mut self,
        transaction: &Transaction,
        block_state: &BlockState,
        data_store: DataStoreWrite,
        tx_logger: &mut TransactionLog,
    ) -> Result<Option<AccountReceipt>, AccountError> {
        gen_account_match!(
            self,
            commit_failed_transaction,
            transaction,
            block_state,
            data_store,
            tx_logger
        )
    }

    fn revert_failed_transaction(
        &mut self,
        transaction: &Transaction,
        block_state: &BlockState,
        receipt: Option<AccountReceipt>,
        data_store: DataStoreWrite,
        tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        gen_account_match!(
            self,
            revert_failed_transaction,
            transaction,
            block_state,
            receipt,
            data_store,
            tx_logger
        )
    }

    fn reserve_balance(
        &self,
        transaction: &Transaction,
        reserved_balance: &mut ReservedBalance,
        block_state: &BlockState,
        data_store: DataStoreRead,
    ) -> Result<(), AccountError> {
        gen_account_match!(
            self,
            reserve_balance,
            transaction,
            reserved_balance,
            block_state,
            data_store
        )
    }

    fn release_balance(
        &self,
        transaction: &Transaction,
        reserved_balance: &mut ReservedBalance,
        data_store: DataStoreRead,
    ) -> Result<(), AccountError> {
        gen_account_match!(
            self,
            release_balance,
            transaction,
            reserved_balance,
            data_store
        )
    }
}

#[cfg(feature = "interaction-traits")]
impl AccountInherentInteraction for Account {
    fn commit_inherent(
        &mut self,
        inherent: &Inherent,
        block_state: &BlockState,
        data_store: DataStoreWrite,
        inherent_logger: &mut InherentLogger,
    ) -> Result<Option<AccountReceipt>, AccountError> {
        gen_account_match!(
            self,
            commit_inherent,
            inherent,
            block_state,
            data_store,
            inherent_logger
        )
    }

    fn revert_inherent(
        &mut self,
        inherent: &Inherent,
        block_state: &BlockState,
        receipt: Option<AccountReceipt>,
        data_store: DataStoreWrite,
        inherent_logger: &mut InherentLogger,
    ) -> Result<(), AccountError> {
        gen_account_match!(
            self,
            revert_inherent,
            inherent,
            block_state,
            receipt,
            data_store,
            inherent_logger
        )
    }
}

#[cfg(feature = "interaction-traits")]
impl AccountPruningInteraction for Account {
    fn can_be_pruned(&self) -> bool {
        gen_account_match!(self, can_be_pruned)
    }

    fn prune(self, data_store: DataStoreRead) -> Option<AccountReceipt> {
        gen_account_match!(self, prune, data_store)
    }

    fn restore(
        ty: AccountType,
        pruned_account: Option<&AccountReceipt>,
        data_store: DataStoreWrite,
    ) -> Result<Account, AccountError> {
        gen_account_type_match!(ty, restore, ty, pruned_account, data_store)
    }
}
