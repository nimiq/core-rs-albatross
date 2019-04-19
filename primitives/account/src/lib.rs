#[macro_use]
extern crate beserial_derive;
#[macro_use]
extern crate log;
extern crate nimiq_hash as hash;
extern crate nimiq_keys as keys;
extern crate nimiq_primitives as primitives;
extern crate nimiq_transaction as transaction;

use std::cmp::Ordering;
use std::io;

use failure::Fail;

use beserial::{Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};
use hash::{Hash, Hasher, HashOutput, SerializeContent};
use keys::Address;
pub use primitives::account::AccountType;
use primitives::coin::Coin;
use transaction::{Transaction, TransactionError};

pub use self::basic_account::BasicAccount;
pub use self::htlc_contract::HashedTimeLockedContract;
pub use self::vesting_contract::VestingContract;
use primitives::coin::CoinParseError;

pub mod basic_account;
pub mod htlc_contract;
pub mod vesting_contract;

pub trait AccountTransactionInteraction: Sized {
    fn new_contract(account_type: AccountType, balance: Coin, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError>;

    fn create(balance: Coin, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError>;

    fn with_incoming_transaction(&self, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError>;

    fn without_incoming_transaction(&self, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError>;

    fn with_outgoing_transaction(&self, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError>;

    fn without_outgoing_transaction(&self, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError>;
}

macro_rules! invoke_account_instance {
    ($on: expr, $name: ident, $( $arg: ident ),*) => {
        match $on {
            Account::Basic(ref account) => Ok(Account::Basic(account.$name($( $arg ),*)?)),
            Account::Vesting(ref account) => Ok(Account::Vesting(account.$name($( $arg ),*)?)),
            Account::HTLC(ref account) => Ok(Account::HTLC(account.$name($( $arg ),*)?)),
        }
    }
}

#[derive(Clone, PartialEq, PartialOrd, Eq, Ord, Debug)]
pub enum Account {
    Basic(BasicAccount),
    Vesting(VestingContract),
    HTLC(HashedTimeLockedContract),
}

impl Account {
    pub const INITIAL: Account = Account::Basic(BasicAccount { balance: Coin::ZERO });

    pub fn new_basic(balance: Coin) -> Account {
        Account::Basic(BasicAccount { balance })
    }

    pub fn account_type(&self) -> AccountType {
        match *self {
            Account::Basic(_) => AccountType::Basic,
            Account::Vesting(_) => AccountType::Vesting,
            Account::HTLC(_) => AccountType::HTLC
        }
    }

    pub fn balance(&self) -> Coin {
        match *self {
            Account::Basic(ref account) => account.balance,
            Account::Vesting(ref account) => account.balance,
            Account::HTLC(ref account) => account.balance
        }
    }

    pub fn is_initial(&self) -> bool {
        match *self {
            Account::Basic(ref account) => account.balance == Coin::ZERO,
            _ => false
        }
    }

    pub fn is_to_be_pruned(&self) -> bool {
        match *self {
            Account::Basic(_) => false,
            _ => self.balance() == Coin::ZERO,
        }
    }

    pub fn balance_add(balance: Coin, value: Coin) -> Result<Coin, AccountError> {
        balance.checked_add(value).ok_or(AccountError::InvalidCoinValue)
    }

    pub fn balance_sub(balance: Coin, value: Coin) -> Result<Coin, AccountError> {
        match balance.checked_sub(value) {
            Some(result) => Ok(result),
            None => Err(AccountError::InsufficientFunds {balance, needed: value})
        }
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
        }
    }
}

impl AccountTransactionInteraction for Account {
    fn new_contract(account_type: AccountType, balance: Coin, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError> {
        match account_type {
            AccountType::Basic => Err(AccountError::InvalidForRecipient),
            AccountType::Vesting => Ok(Account::Vesting(VestingContract::create(balance, transaction, block_height)?)),
            AccountType::HTLC => Ok(Account::HTLC(HashedTimeLockedContract::create(balance, transaction, block_height)?))
        }
    }

    fn create(_balance: Coin, _transaction: &Transaction, _block_height: u32) -> Result<Self, AccountError> {
        Err(AccountError::InvalidForRecipient)
    }

    fn with_incoming_transaction(&self, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError> {
        invoke_account_instance!(*self, with_incoming_transaction, transaction, block_height)
    }

    fn without_incoming_transaction(&self, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError> {
        invoke_account_instance!(*self, without_incoming_transaction, transaction, block_height)
    }

    fn with_outgoing_transaction(&self, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError> {
        // Check account balance.
        // This assumes that transaction.value + transaction.fee does not overflow.
        let balance = self.balance();
        let value = transaction.value.checked_add(transaction.fee).ok_or(AccountError::InvalidCoinValue)?;
        if balance < value {
            return Err(AccountError::InsufficientFunds { balance, needed: value});
        }

        invoke_account_instance!(*self, with_outgoing_transaction, transaction, block_height)
    }

    fn without_outgoing_transaction(&self, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError> {
        invoke_account_instance!(*self, without_outgoing_transaction, transaction, block_height)
    }
}

#[derive(Clone, Eq, Debug, Serialize, Deserialize)]
pub struct PrunedAccount {
    pub address: Address,
    pub account: Account,
}

impl SerializeContent for PrunedAccount {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> { Ok(self.serialize(writer)?) }
}

impl Hash for PrunedAccount {
    fn hash<H: HashOutput>(&self) -> H  {
        let h = H::Builder::default();
        self.serialize_content(&mut vec![]).unwrap();
        h.finish()
    }
}

impl Ord for PrunedAccount {
    fn cmp(&self, other: &PrunedAccount) -> Ordering {
        self.address.cmp(&other.address)
    }
}

impl PartialOrd for PrunedAccount {
    fn partial_cmp(&self, other: &PrunedAccount) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for PrunedAccount {
    fn eq(&self, other: &PrunedAccount) -> bool {
        self.address == other.address
    }
}


#[derive(Clone, Debug, Eq, Fail, PartialEq)]
pub enum AccountError {
    #[fail(display = "Insufficient funds: needed {}, but has balance {}", needed, balance)]
    InsufficientFunds { needed: Coin, balance: Coin },
    // TODO: This still needs to be displayed as debug
    #[fail(display = "Type mismatch: expected {}, but got {}", expected, got)]
    TypeMismatch { expected: AccountType, got: AccountType },
    #[fail(display = "Invalid signature")]
    InvalidSignature,
    #[fail(display = "Invalid for sender")]
    InvalidForSender,
    #[fail(display = "Invalid for recipient")]
    InvalidForRecipient,
    #[fail(display = "Invalid pruning")]
    InvalidPruning,
    #[fail(display = "Invalid serialization")]
    InvalidSerialization(#[cause] SerializingError),
    #[fail(display = "Invalid transaction")]
    InvalidTransaction(#[cause] TransactionError),
    #[fail(display = "Invalid coin value")]
    InvalidCoinValue,
    #[fail(display = "Accounts hash mismatch")]
    AccountsHashMismatch, // XXX This doesn't really belong here
}

impl From<SerializingError> for AccountError {
    fn from(e: SerializingError) -> Self {
        AccountError::InvalidSerialization(e)
    }
}

impl From<TransactionError> for AccountError {
    fn from(e: TransactionError) -> Self {
        AccountError::InvalidTransaction(e)
    }
}

impl From<CoinParseError> for AccountError {
    fn from(_: CoinParseError) -> Self {
        AccountError::InvalidCoinValue
    }
}
