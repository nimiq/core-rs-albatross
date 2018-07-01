pub mod tree;
pub mod basic_account;
pub mod htlc_contract;
pub mod vesting_contract;

use beserial::{Deserialize, Serialize, WriteBytesExt, ReadBytesExt};
use consensus::base::primitive::Address;
use consensus::base::primitive::hash::{Hash, SerializeContent};
pub use self::basic_account::BasicAccount;
pub use self::htlc_contract::HashedTimeLockedContract;
pub use self::vesting_contract::VestingContract;
use std::io;

#[derive(Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum AccountType {
    Basic = 0,
    Vesting = 1,
    HTLC = 2,
}

#[derive(Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Debug)]
pub enum Account {
    Basic(BasicAccount),
    Vesting(VestingContract),
    HTLC(HashedTimeLockedContract),
}

impl Account {
    pub fn is_initial(&self) -> bool {
        if let Account::Basic(ref account) = *self {
            return account.balance == 0;
        } else {
            return false;
        }
    }

    fn account_type(&self) -> AccountType {
        return match *self {
            Account::Basic(_) => AccountType::Basic,
            Account::Vesting(_) => AccountType::Vesting,
            Account::HTLC(_) => AccountType::HTLC,
        };
    }

    pub fn balance(&self) -> u64 {
        return match *self {
            Account::Basic(ref account) => account.balance,
            Account::Vesting(ref account) => account.balance,
            Account::HTLC(ref account) => account.balance,
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
            },
            Account::Vesting(ref account) => {
                size += Serialize::serialize(&account, writer)?;
            },
            Account::HTLC(ref account) => {
                size += Serialize::serialize(&account, writer)?;
            },
        }

        return Ok(size);
    }

    fn serialized_size(&self) -> usize {
        let mut size = /*type*/ 1;

        match *self {
            Account::Basic(ref account) => {
                size += Serialize::serialized_size(&account);
            },
            Account::Vesting(ref account) => {
                size += Serialize::serialized_size(&account);
            },
            Account::HTLC(ref account) => {
                size += Serialize::serialized_size(&account);
            },
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
            },
            AccountType::Vesting => {
                let account: VestingContract = Deserialize::deserialize(reader)?;
                return Ok(Account::Vesting(account));
            },
            AccountType::HTLC => {
                let account: HashedTimeLockedContract = Deserialize::deserialize(reader)?;
                return Ok(Account::HTLC(account));
            },
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
