use beserial::{Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};
use nimiq_primitives::account::AccountType;
use nimiq_primitives::coin::Coin;
use nimiq_transaction::Transaction;

use crate::interaction_traits::{AccountInherentInteraction, AccountTransactionInteraction};
use crate::{
    AccountError, BasicAccount, HashedTimeLockedContract, Inherent, StakingContract,
    VestingContract,
};

macro_rules! invoke_account_instance {
    ($on: expr, $name: ident, $( $arg: ident ),*) => {
        match $on {
            Account::Basic(account) => account.$name($( $arg ),*),
            Account::Vesting(account) => account.$name($( $arg ),*),
            Account::HTLC(account) => account.$name($( $arg ),*),
            Account::Staking(account) => account.$name($( $arg ),*),
        }
    }
}

#[derive(Clone, PartialEq, PartialOrd, Eq, Ord, Debug)]
#[cfg_attr(feature = "serde-derive", derive(serde::Serialize, serde::Deserialize))]
pub enum Account {
    Basic(BasicAccount),
    Vesting(VestingContract),
    HTLC(HashedTimeLockedContract),
    #[cfg_attr(feature = "serde-derive", serde(skip))]
    Staking(StakingContract),
}

impl Account {
    pub const INITIAL: Account = Account::Basic(BasicAccount {
        balance: Coin::ZERO,
    });

    pub fn new_basic(balance: Coin) -> Account {
        Account::Basic(BasicAccount { balance })
    }

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

    pub fn is_to_be_pruned(&self) -> bool {
        match *self {
            Account::Basic(_) | Account::Staking(_) => false,
            _ => self.balance() == Coin::ZERO,
        }
    }

    pub fn balance_add(balance: Coin, value: Coin) -> Result<Coin, AccountError> {
        balance
            .checked_add(value)
            .ok_or(AccountError::InvalidCoinValue)
    }

    pub fn balance_sub(balance: Coin, value: Coin) -> Result<Coin, AccountError> {
        match balance.checked_sub(value) {
            Some(result) => Ok(result),
            None => Err(AccountError::InsufficientFunds {
                balance,
                needed: value,
            }),
        }
    }

    pub fn balance_sufficient(balance: Coin, value: Coin) -> Result<(), AccountError> {
        if balance < value {
            Err(AccountError::InsufficientFunds {
                balance,
                needed: value,
            })
        } else {
            Ok(())
        }
    }

    pub fn check_incoming_transaction(
        &self,
        transaction: &Transaction,
        block_height: u32,
        time: u64,
    ) -> Result<(), AccountError> {
        match self {
            Account::Basic(_) => {
                BasicAccount::check_incoming_transaction(transaction, block_height, time)
            }
            Account::Vesting(_) => {
                VestingContract::check_incoming_transaction(transaction, block_height, time)
            }
            Account::HTLC(_) => HashedTimeLockedContract::check_incoming_transaction(
                transaction,
                block_height,
                time,
            ),
            Account::Staking(_) => {
                StakingContract::check_incoming_transaction(transaction, block_height, time)
            }
        }
    }
}

impl AccountTransactionInteraction for Account {
    fn new_contract(
        account_type: AccountType,
        balance: Coin,
        transaction: &Transaction,
        block_height: u32,
        time: u64,
    ) -> Result<Self, AccountError> {
        match account_type {
            AccountType::Basic => Err(AccountError::InvalidForRecipient),
            AccountType::Vesting => Ok(Account::Vesting(VestingContract::create(
                balance,
                transaction,
                block_height,
                time,
            )?)),
            AccountType::HTLC => Ok(Account::HTLC(HashedTimeLockedContract::create(
                balance,
                transaction,
                block_height,
                time,
            )?)),
            AccountType::Staking => Err(AccountError::InvalidForRecipient),
            AccountType::Reward => Err(AccountError::InvalidForRecipient),
        }
    }

    fn create(
        _balance: Coin,
        _transaction: &Transaction,
        _block_height: u32,
        _time: u64,
    ) -> Result<Self, AccountError> {
        Err(AccountError::InvalidForRecipient)
    }

    fn check_incoming_transaction(
        _transaction: &Transaction,
        _block_height: u32,
        _time: u64,
    ) -> Result<(), AccountError> {
        // This method must be called on specific types instead.
        Err(AccountError::InvalidForRecipient)
    }

    fn commit_incoming_transaction(
        &mut self,
        transaction: &Transaction,
        block_height: u32,
        time: u64,
    ) -> Result<Option<Vec<u8>>, AccountError> {
        self.check_incoming_transaction(transaction, block_height, time)?;
        invoke_account_instance!(
            self,
            commit_incoming_transaction,
            transaction,
            block_height,
            time
        )
    }

    fn revert_incoming_transaction(
        &mut self,
        transaction: &Transaction,
        block_height: u32,
        time: u64,
        receipt: Option<&Vec<u8>>,
    ) -> Result<(), AccountError> {
        invoke_account_instance!(
            self,
            revert_incoming_transaction,
            transaction,
            block_height,
            time,
            receipt
        )
    }

    fn check_outgoing_transaction(
        &self,
        transaction: &Transaction,
        block_height: u32,
        time: u64,
    ) -> Result<(), AccountError> {
        invoke_account_instance!(
            self,
            check_outgoing_transaction,
            transaction,
            block_height,
            time
        )
    }

    fn commit_outgoing_transaction(
        &mut self,
        transaction: &Transaction,
        block_height: u32,
        time: u64,
    ) -> Result<Option<Vec<u8>>, AccountError> {
        invoke_account_instance!(
            self,
            commit_outgoing_transaction,
            transaction,
            block_height,
            time
        )
    }

    fn revert_outgoing_transaction(
        &mut self,
        transaction: &Transaction,
        block_height: u32,
        time: u64,
        receipt: Option<&Vec<u8>>,
    ) -> Result<(), AccountError> {
        invoke_account_instance!(
            self,
            revert_outgoing_transaction,
            transaction,
            block_height,
            time,
            receipt
        )
    }
}

impl AccountInherentInteraction for Account {
    fn check_inherent(
        &self,
        inherent: &Inherent,
        block_height: u32,
        time: u64,
    ) -> Result<(), AccountError> {
        invoke_account_instance!(self, check_inherent, inherent, block_height, time)
    }

    fn commit_inherent(
        &mut self,
        inherent: &Inherent,
        block_height: u32,
        time: u64,
    ) -> Result<Option<Vec<u8>>, AccountError> {
        invoke_account_instance!(self, commit_inherent, inherent, block_height, time)
    }

    fn revert_inherent(
        &mut self,
        inherent: &Inherent,
        block_height: u32,
        time: u64,
        receipt: Option<&Vec<u8>>,
    ) -> Result<(), AccountError> {
        invoke_account_instance!(self, revert_inherent, inherent, block_height, time, receipt)
    }
}

impl Serialize for Account {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size: usize = 0;
        size += Serialize::serialize(&self.account_type(), writer)?;

        match *self {
            Account::Basic(ref account) => {
                size += Serialize::serialize(&account, writer)?;
            }
            Account::Vesting(ref account) => {
                size += Serialize::serialize(&account, writer)?;
            }
            Account::HTLC(ref account) => {
                size += Serialize::serialize(&account, writer)?;
            }
            Account::Staking(ref account) => {
                size += Serialize::serialize(&account, writer)?;
            }
        }

        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size = /*type*/ 1;

        match *self {
            Account::Basic(ref account) => {
                size += Serialize::serialized_size(&account);
            }
            Account::Vesting(ref account) => {
                size += Serialize::serialized_size(&account);
            }
            Account::HTLC(ref account) => {
                size += Serialize::serialized_size(&account);
            }
            Account::Staking(ref account) => {
                size += Serialize::serialized_size(&account);
            }
        }

        size
    }
}

impl Deserialize for Account {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let account_type: AccountType = Deserialize::deserialize(reader)?;

        match account_type {
            AccountType::Basic => {
                let account: BasicAccount = Deserialize::deserialize(reader)?;
                Ok(Account::Basic(account))
            }
            AccountType::Vesting => {
                let account: VestingContract = Deserialize::deserialize(reader)?;
                Ok(Account::Vesting(account))
            }
            AccountType::HTLC => {
                let account: HashedTimeLockedContract = Deserialize::deserialize(reader)?;
                Ok(Account::HTLC(account))
            }
            AccountType::Staking => {
                let account: StakingContract = Deserialize::deserialize(reader)?;
                Ok(Account::Staking(account))
            }
            AccountType::Reward => unimplemented!(),
        }
    }
}
