use std::{fmt::Debug, io};

use nimiq_database_value::{FromDatabaseValue, IntoDatabaseValue};
use nimiq_primitives::{account::FailReason, trie::trie_diff::RevertTrieDiff};
use nimiq_serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountReceipt(pub Vec<u8>);

impl From<Vec<u8>> for AccountReceipt {
    fn from(val: Vec<u8>) -> Self {
        AccountReceipt(val)
    }
}

#[macro_export]
macro_rules! convert_receipt {
    ($t: ty) => {
        impl TryFrom<AccountReceipt> for $t {
            type Error = AccountError;

            fn try_from(value: AccountReceipt) -> Result<Self, Self::Error> {
                <$t>::try_from(&value)
            }
        }

        impl TryFrom<&AccountReceipt> for $t {
            type Error = AccountError;

            fn try_from(value: &AccountReceipt) -> Result<Self, Self::Error> {
                Self::deserialize_from_vec(&value.0[..])
                    .map_err(|e| AccountError::InvalidSerialization(e))
            }
        }

        impl From<$t> for AccountReceipt {
            fn from(value: $t) -> Self {
                AccountReceipt::from(value.serialize_to_vec())
            }
        }
    };
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransactionReceipt {
    pub sender_receipt: Option<AccountReceipt>,
    pub recipient_receipt: Option<AccountReceipt>,
    pub pruned_account: Option<AccountReceipt>,
}

pub type InherentReceipt = Option<AccountReceipt>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(bound = "T: Clone + Debug + Serialize + Deserialize")]
#[repr(u8)]
pub enum OperationReceipt<T: Clone + Debug + Serialize + Deserialize> {
    Ok(T),
    Err(T, FailReason),
}

pub type TransactionOperationReceipt = OperationReceipt<TransactionReceipt>;
pub type InherentOperationReceipt = OperationReceipt<InherentReceipt>;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Receipts {
    pub transactions: Vec<TransactionOperationReceipt>,
    pub inherents: Vec<InherentOperationReceipt>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[repr(u8)]
pub enum RevertInfo {
    Receipts(Receipts),
    Diff(RevertTrieDiff),
}

impl From<Receipts> for RevertInfo {
    fn from(receipts: Receipts) -> RevertInfo {
        RevertInfo::Receipts(receipts)
    }
}

impl From<RevertTrieDiff> for RevertInfo {
    fn from(diff: RevertTrieDiff) -> RevertInfo {
        RevertInfo::Diff(diff)
    }
}

// TODO Implement sparse serialization for Receipts

impl IntoDatabaseValue for RevertInfo {
    fn database_byte_size(&self) -> usize {
        self.serialized_size()
    }

    fn copy_into_database(&self, mut bytes: &mut [u8]) {
        self.serialize_to_writer(&mut bytes).unwrap();
    }
}

impl FromDatabaseValue for RevertInfo {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self>
    where
        Self: Sized,
    {
        Deserialize::deserialize_from_vec(bytes)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))
    }
}
