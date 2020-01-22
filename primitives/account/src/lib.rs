#[macro_use]
extern crate beserial_derive;
#[macro_use]
extern crate log;
extern crate lazy_static;
extern crate nimiq_bls as bls;
extern crate nimiq_utils as utils;
extern crate nimiq_hash as hash;
extern crate nimiq_keys as keys;
extern crate nimiq_primitives as primitives;
extern crate nimiq_transaction as transaction;
extern crate nimiq_vrf as vrf;

use std::cmp::Ordering;
use std::io;
use std::convert::TryFrom;

use failure::Fail;

use beserial::{Deserialize, DeserializeWithLength, ReadBytesExt, Serialize, SerializeWithLength, SerializingError, WriteBytesExt};
use hash::{Hash, Hasher, HashOutput, SerializeContent};
use keys::Address;
pub use primitives::account::AccountType;
use primitives::coin::{Coin, CoinParseError};
use transaction::{Transaction, TransactionError};

use crate::inherent::{AccountInherentInteraction};

pub use self::basic_account::BasicAccount;
pub use self::htlc_contract::HashedTimeLockedContract;
pub use self::inherent::{Inherent, InherentType};
pub use self::staking_contract::{StakingContract};
pub use self::vesting_contract::VestingContract;

pub mod inherent;
pub mod basic_account;
pub mod htlc_contract;
pub mod vesting_contract;
pub mod staking_contract;

/// Given the actual sender/receiver account (types), this checks and changes the accounts' states.
/// TODO: `check_incoming_transaction` is not allowed to depend on the account's state.
pub trait AccountTransactionInteraction: Sized {
    fn new_contract(account_type: AccountType, balance: Coin, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError>;

    fn create(balance: Coin, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError>;


    /// Incoming transaction checks must not depend on the account itself and may only be static on the transaction.
    fn check_incoming_transaction(transaction: &Transaction, block_height: u32) -> Result<(), AccountError>;

    fn commit_incoming_transaction(&mut self, transaction: &Transaction, block_height: u32) -> Result<Option<Vec<u8>>, AccountError>;

    fn revert_incoming_transaction(&mut self, transaction: &Transaction, block_height: u32, receipt: Option<&Vec<u8>>) -> Result<(), AccountError>;


    /// Outgoing transaction checks are usually called from `commit_outgoint_transaction` and may depend on the account's state.
    fn check_outgoing_transaction(&self, transaction: &Transaction, block_height: u32) -> Result<(), AccountError>;

    fn commit_outgoing_transaction(&mut self, transaction: &Transaction, block_height: u32) -> Result<Option<Vec<u8>>, AccountError>;

    fn revert_outgoing_transaction(&mut self, transaction: &Transaction, block_height: u32, receipt: Option<&Vec<u8>>) -> Result<(), AccountError>;
}

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

pub trait AccountsTreeLeave: Serialize + Deserialize + Clone {
    fn is_initial(&self) -> bool;
}

#[derive(Clone, PartialEq, PartialOrd, Eq, Ord, Debug)]
pub enum Account {
    Basic(BasicAccount),
    Vesting(VestingContract),
    HTLC(HashedTimeLockedContract),
    Staking(StakingContract),
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
        balance.checked_add(value).ok_or(AccountError::InvalidCoinValue)
    }

    pub fn balance_sub(balance: Coin, value: Coin) -> Result<Coin, AccountError> {
        match balance.checked_sub(value) {
            Some(result) => Ok(result),
            None => Err(AccountError::InsufficientFunds {balance, needed: value})
        }
    }

    pub fn balance_sufficient(balance: Coin, value: Coin) -> Result<(), AccountError> {
        if balance < value {
            Err(AccountError::InsufficientFunds {balance, needed: value})
        } else {
            Ok(())
        }
    }

    pub fn check_incoming_transaction(&self, transaction: &Transaction, block_height: u32) -> Result<(), AccountError> {
        match self {
            Account::Basic(_) => BasicAccount::check_incoming_transaction(transaction, block_height),
            Account::Vesting(_) => VestingContract::check_incoming_transaction(transaction, block_height),
            Account::HTLC(_) => HashedTimeLockedContract::check_incoming_transaction(transaction, block_height),
            Account::Staking(_) => StakingContract::check_incoming_transaction(transaction, block_height),
        }
    }
}

impl AccountsTreeLeave for Account {
    fn is_initial(&self) -> bool {
        match *self {
            Account::Basic(ref account) => account.balance == Coin::ZERO,
            _ => false
        }
    }
}

impl AccountTransactionInteraction for Account {
    fn new_contract(account_type: AccountType, balance: Coin, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError> {
        match account_type {
            AccountType::Basic => Err(AccountError::InvalidForRecipient),
            AccountType::Vesting => Ok(Account::Vesting(VestingContract::create(balance, transaction, block_height)?)),
            AccountType::HTLC => Ok(Account::HTLC(HashedTimeLockedContract::create(balance, transaction, block_height)?)),
            AccountType::Staking => Err(AccountError::InvalidForRecipient),
        }
    }

    fn create(_balance: Coin, _transaction: &Transaction, _block_height: u32) -> Result<Self, AccountError> {
        Err(AccountError::InvalidForRecipient)
    }

    fn check_incoming_transaction(_transaction: &Transaction, _block_height: u32) -> Result<(), AccountError> {
        // This method must be called on specific types instead.
        Err(AccountError::InvalidForRecipient)
    }

    fn commit_incoming_transaction(&mut self, transaction: &Transaction, block_height: u32) -> Result<Option<Vec<u8>>, AccountError> {
        self.check_incoming_transaction(transaction, block_height)?;
        invoke_account_instance!(self, commit_incoming_transaction, transaction, block_height)
    }

    fn revert_incoming_transaction(&mut self, transaction: &Transaction, block_height: u32, receipt: Option<&Vec<u8>>) -> Result<(), AccountError> {
        invoke_account_instance!(self, revert_incoming_transaction, transaction, block_height, receipt)
    }

    fn check_outgoing_transaction(&self, transaction: &Transaction, block_height: u32) -> Result<(), AccountError> {
        invoke_account_instance!(self, check_outgoing_transaction, transaction, block_height)
    }

    fn commit_outgoing_transaction(&mut self, transaction: &Transaction, block_height: u32) -> Result<Option<Vec<u8>>, AccountError> {
        invoke_account_instance!(self, commit_outgoing_transaction, transaction, block_height)
    }

    fn revert_outgoing_transaction(&mut self, transaction: &Transaction, block_height: u32, receipt: Option<&Vec<u8>>) -> Result<(), AccountError> {
        invoke_account_instance!(self, revert_outgoing_transaction, transaction, block_height, receipt)
    }
}

impl AccountInherentInteraction for Account {
    fn check_inherent(&self, inherent: &Inherent, block_height: u32) -> Result<(), AccountError> {
        invoke_account_instance!(self, check_inherent, inherent, block_height)
    }

    fn commit_inherent(&mut self, inherent: &Inherent, block_height: u32) -> Result<Option<Vec<u8>>, AccountError> {
        invoke_account_instance!(self, commit_inherent, inherent, block_height)
    }

    fn revert_inherent(&mut self, inherent: &Inherent, block_height: u32, receipt: Option<&Vec<u8>>) -> Result<(), AccountError> {
        invoke_account_instance!(self, revert_inherent, inherent, block_height, receipt)
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
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum ReceiptType {
    PrunedAccount = 0,
    Transaction = 1,
    Inherent = 2,
}

#[derive(Clone, Eq, PartialEq, Debug)]
pub enum Receipt {
    PrunedAccount(PrunedAccount),
    Transaction {
        index: u16,
        sender: bool, // A bit inefficient.
        data: Vec<u8>,
    },
    Inherent {
        index: u16,
        data: Vec<u8>,
        pre_transactions: bool,
    },
}

impl Receipt {
    pub fn receipt_type(&self) -> ReceiptType {
        match self {
            Receipt::PrunedAccount(_) => ReceiptType::PrunedAccount,
            Receipt::Transaction {..} => ReceiptType::Transaction,
            Receipt::Inherent {..} => ReceiptType::Inherent,
        }
    }
}

impl Serialize for Receipt {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = self.receipt_type().serialize(writer)?;
        match self {
            Receipt::PrunedAccount(pruned_account) => size += pruned_account.serialize(writer)?,
            Receipt::Transaction { index, sender, data } => {
                size += index.serialize(writer)?;
                size += sender.serialize(writer)?;
                size += SerializeWithLength::serialize::<u16, W>(data, writer)?;
            },
            Receipt::Inherent { index, data, pre_transactions } => {
                size += index.serialize(writer)?;
                size += SerializeWithLength::serialize::<u16, W>(data, writer)?;
                size += pre_transactions.serialize(writer)?;
            },
        }
        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size = self.receipt_type().serialized_size();
        match self {
            Receipt::PrunedAccount(pruned_account) => size += pruned_account.serialized_size(),
            Receipt::Transaction { index, sender, data } => {
                size += index.serialized_size();
                size += sender.serialized_size();
                size += SerializeWithLength::serialized_size::<u16>(data);
            },
            Receipt::Inherent { index, data, pre_transactions } => {
                size += index.serialized_size();
                size += SerializeWithLength::serialized_size::<u16>(data);
                size += pre_transactions.serialized_size();
            },
        }
        size
    }
}

impl Deserialize for Receipt {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let ty: ReceiptType = Deserialize::deserialize(reader)?;
        match ty {
            ReceiptType::PrunedAccount => Ok(Receipt::PrunedAccount(Deserialize::deserialize(reader)?)),
            ReceiptType::Transaction => Ok(Receipt::Transaction {
                index: Deserialize::deserialize(reader)?,
                sender: Deserialize::deserialize(reader)?,
                data: DeserializeWithLength::deserialize::<u16, R>(reader)?,
            }),
            ReceiptType::Inherent => Ok(Receipt::Inherent {
                index: Deserialize::deserialize(reader)?,
                data: DeserializeWithLength::deserialize::<u16, R>(reader)?,
                pre_transactions: Deserialize::deserialize(reader)?,
            }),
        }
    }
}

impl SerializeContent for Receipt {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> { Ok(self.serialize(writer)?) }
}

#[allow(clippy::derive_hash_xor_eq)] // TODO: Shouldn't be necessary
impl Hash for Receipt {
    fn hash<H: HashOutput>(&self) -> H  {
        let h = H::Builder::default();
        self.serialize_content(&mut vec![]).unwrap();
        h.finish()
    }
}

impl Ord for Receipt {
    fn cmp(&self, other: &Receipt) -> Ordering {
        match (self, other) {
            (Receipt::PrunedAccount(a), Receipt::PrunedAccount(b)) => a.cmp(b),
            (Receipt::Transaction { index: index_a, sender: sender_a, data: data_a }, Receipt::Transaction { index: index_b, sender: sender_b, data: data_b }) => {
                // Ensure order is the same as when processing the block.
                sender_a.cmp(sender_b).reverse()
                    .then_with(|| index_a.cmp(index_b))
                    .then_with(|| data_a.cmp(data_b))
            },
            (Receipt::Inherent { index: index_a, data: data_a, pre_transactions: pre_transactions_a }, Receipt::Inherent { index: index_b, data: data_b, pre_transactions: pre_transactions_b }) => {
                pre_transactions_a.cmp(pre_transactions_b).reverse()
                    .then_with(|| index_a.cmp(index_b))
                    .then_with(|| data_a.cmp(data_b))
            }
            (a, b) => a.receipt_type().cmp(&b.receipt_type())
        }
    }
}

impl PartialOrd for Receipt {
    fn partial_cmp(&self, other: &Receipt) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Default, Clone, Eq, PartialEq, Ord, PartialOrd, Debug, Serialize, Deserialize)]
pub struct Receipts {
    #[beserial(len_type(u16))]
    pub receipts: Vec<Receipt>,
}

impl Receipts {
    pub fn sort(&mut self) {
        self.receipts.sort();
    }

    pub fn len(&self) -> usize {
        self.receipts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.receipts.is_empty()
    }
}

impl From<Vec<Receipt>> for Receipts {
    fn from(receipts: Vec<Receipt>) -> Self {
        Receipts { receipts }
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
    #[fail(display = "Invalid for target")]
    InvalidForTarget,
    #[fail(display = "Invalid receipt")]
    InvalidReceipt,
    #[fail(display = "Invalid serialization")]
    InvalidSerialization(#[cause] SerializingError),
    #[fail(display = "Invalid transaction")]
    InvalidTransaction(#[cause] TransactionError),
    #[fail(display = "Invalid coin value")]
    InvalidCoinValue,
    #[fail(display = "Invalid inherent")]
    InvalidInherent,
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


/// A small wrapper over a list of accounts with addresses. This is only used to have method
/// of serializing and deserializing the genesis accounts.
pub struct AccountsList(pub Vec<(Address, Account)>);

impl Deserialize for AccountsList {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let count: u16 = Deserialize::deserialize(reader)?;
        let mut accounts: Vec<(Address, Account)> = Vec::new();
        for _ in 0..count {
            accounts.push((
                Deserialize::deserialize(reader)?,
                Deserialize::deserialize(reader)?
            ));
        }
        Ok(Self(accounts))
    }
}

impl Serialize for AccountsList {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = 0;
        let count: u16 = u16::try_from(self.0.len())
            .map_err(|_| SerializingError::Overflow)?;
        size += count.serialize(writer)?;
        for (address, account) in self.0.iter() {
            size += address.serialize(writer)?;
            size += account.serialize(writer)?;
        }
        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size = 2; // count as u16
        for (address, account) in self.0.iter() {
            size += address.serialized_size();
            size += account.serialized_size();
        }
        size
    }
}
