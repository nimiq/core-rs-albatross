pub mod tree;
pub mod basic_account;
pub mod htlc_contract;
pub mod vesting_contract;
pub mod accounts;

use beserial::{Deserialize, Serialize, WriteBytesExt, ReadBytesExt};
use super::transaction::Transaction;
use consensus::base::primitive::Address;
use consensus::base::primitive::hash::{Hash, SerializeContent};
use std::io;

pub use self::basic_account::BasicAccount;
pub use self::htlc_contract::HashedTimeLockedContract;
pub use self::vesting_contract::VestingContract;
pub use self::accounts::Accounts;

#[derive(Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum AccountType {
    Basic = 0,
    Vesting = 1,
    HTLC = 2,
}

#[derive(Clone, PartialEq, PartialOrd, Eq, Ord, Debug)]
pub enum Account {
    Basic(BasicAccount),
    Vesting(VestingContract),
    HTLC(HashedTimeLockedContract),
}

impl Account {
    const INITIAL: Account = Account::Basic(BasicAccount { balance: 0 });

    pub fn new_basic(balance: u64) -> Account {
        return Account::Basic(BasicAccount { balance });
    }

    pub fn new_contract(account_type: AccountType, balance: u64, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError> {
        return match account_type {
            AccountType::Basic => Err(AccountError("Invalid contract type".to_string())),
            AccountType::Vesting => Ok(Account::Vesting(VestingContract::create(balance, transaction, block_height)?)),
            AccountType::HTLC => Ok(Account::HTLC(HashedTimeLockedContract::create(balance, transaction, block_height)?))
        };
    }

    pub fn verify_incoming_transaction(account_type: AccountType, transaction: &Transaction, block_height: u32) -> bool {
        return match account_type {
            AccountType::Basic => BasicAccount::verify_incoming_transaction(transaction, block_height),
            AccountType::Vesting => VestingContract::verify_incoming_transaction(transaction, block_height),
            AccountType::HTLC => HashedTimeLockedContract::verify_incoming_transaction(transaction, block_height)
        }
    }

    pub fn verify_outgoing_transaction(account_type: AccountType, transaction: &Transaction, block_height: u32) -> bool {
        return match account_type {
            AccountType::Basic => BasicAccount::verify_outgoing_transaction(transaction, block_height),
            AccountType::Vesting => VestingContract::verify_outgoing_transaction(transaction, block_height),
            AccountType::HTLC => HashedTimeLockedContract::verify_outgoing_transaction(transaction, block_height)
        }
    }

    pub fn with_incoming_transaction(&self, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError> {
        return match *self {
            Account::Basic(ref account) => Ok(Account::Basic(account.with_incoming_transaction(transaction, block_height)?)),
            Account::Vesting(ref account) => Ok(Account::Vesting(account.with_incoming_transaction(transaction, block_height)?)),
            Account::HTLC(ref account) => Ok(Account::HTLC(account.with_incoming_transaction(transaction, block_height)?))
        };
    }

    pub fn without_incoming_transaction(&self, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError> {
        return match *self {
            Account::Basic(ref account) => Ok(Account::Basic(account.without_incoming_transaction(transaction, block_height)?)),
            Account::Vesting(ref account) => Ok(Account::Vesting(account.without_incoming_transaction(transaction, block_height)?)),
            Account::HTLC(ref account) => Ok(Account::HTLC(account.without_incoming_transaction(transaction, block_height)?))
        };
    }

    pub fn with_outgoing_transaction(&self, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError> {
        // Check account balance.
        // !!! This assumes that transaction.value + transaction.fee does not overflow. !!!
        let balance = self.balance();
        if balance < transaction.value + transaction.fee {
            return Err(AccountError("Insufficient funds".to_string()));
        }

        return match *self {
            Account::Basic(ref account) => Ok(Account::Basic(account.with_outgoing_transaction(transaction, block_height)?)),
            Account::Vesting(ref account) => Ok(Account::Vesting(account.with_outgoing_transaction(transaction, block_height)?)),
            Account::HTLC(ref account) => Ok(Account::HTLC(account.with_outgoing_transaction(transaction, block_height)?))
        };
    }

    pub fn without_outgoing_transaction(&self, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError> {
        return match *self {
            Account::Basic(ref account) => Ok(Account::Basic(account.without_outgoing_transaction(transaction, block_height)?)),
            Account::Vesting(ref account) => Ok(Account::Vesting(account.without_outgoing_transaction(transaction, block_height)?)),
            Account::HTLC(ref account) => Ok(Account::HTLC(account.without_outgoing_transaction(transaction, block_height)?))
        };
    }

    fn account_type(&self) -> AccountType {
        return match *self {
            Account::Basic(_) => AccountType::Basic,
            Account::Vesting(_) => AccountType::Vesting,
            Account::HTLC(_) => AccountType::HTLC
        };
    }

    pub fn balance(&self) -> u64 {
        return match *self {
            Account::Basic(ref account) => account.balance,
            Account::Vesting(ref account) => account.balance,
            Account::HTLC(ref account) => account.balance
        };
    }

    pub fn is_initial(&self) -> bool {
        if let Account::Basic(ref account) = *self {
            return account.balance == 0;
        } else {
            return false;
        }
    }

    pub fn is_to_be_pruned(&self) -> bool {
        return match *self {
            Account::Basic(_) => false,
            Account::Vesting(ref account) => account.balance == 0,
            Account::HTLC(ref account) => account.balance == 0
        };
    }

    pub fn balance_add(balance: u64, value: u64) -> Result<u64, AccountError> {
        return match balance.checked_add(value) {
            Some(result) => Ok(result),
            None => Err(AccountError("Balance overflow (add)".to_string()))
        };
    }

    pub fn balance_sub(balance: u64, value: u64) -> Result<u64, AccountError> {
        return match balance.checked_sub(value) {
            Some(result) => Ok(result),
            None => Err(AccountError("Balance overflow (sub)".to_string()))
        };
    }
}

impl Serialize for Account {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> io::Result<usize> {
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

        return Ok(size);
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

        return size;
    }
}

impl Deserialize for Account {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> io::Result<Self> {
        let account_type: AccountType = Deserialize::deserialize(reader)?;

        match account_type {
            AccountType::Basic => {
                let account: BasicAccount = Deserialize::deserialize(reader)?;
                return Ok(Account::Basic(account));
            }
            AccountType::Vesting => {
                let account: VestingContract = Deserialize::deserialize(reader)?;
                return Ok(Account::Vesting(account));
            }
            AccountType::HTLC => {
                let account: HashedTimeLockedContract = Deserialize::deserialize(reader)?;
                return Ok(Account::HTLC(account));
            }
        }
    }
}

#[derive(Clone, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
pub struct PrunedAccount {
    address: Address,
    account: Account,
}

impl SerializeContent for PrunedAccount {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> { self.serialize(writer) }
}

impl Hash for PrunedAccount {}

#[derive(Debug, Clone)]
pub struct AccountError(String);
