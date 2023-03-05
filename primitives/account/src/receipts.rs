use std::fmt::Debug;
use std::io;

use beserial::{Deserialize, Serialize};
use nimiq_database_value::{FromDatabaseValue, IntoDatabaseValue};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountReceipt(#[beserial(len_type(u16))] pub Vec<u8>);

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
                Self::deserialize(&mut &value.0[..])
                    .map_err(|e| AccountError::InvalidSerialization(e))
            }
        }

        impl Into<AccountReceipt> for $t {
            fn into(self) -> AccountReceipt {
                AccountReceipt::from(self.serialize_to_vec())
            }
        }
    };
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TransactionReceipt {
    pub sender_receipt: Option<AccountReceipt>,
    pub recipient_receipt: Option<AccountReceipt>,
    pub pruned_account: Option<AccountReceipt>,
}

pub type InherentReceipt = Option<AccountReceipt>;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum OperationReceipt<T: Clone + Debug + Serialize + Deserialize> {
    Ok(T),
    Err(T),
}

pub type TransactionOperationReceipt = OperationReceipt<TransactionReceipt>;
pub type InherentOperationReceipt = OperationReceipt<InherentReceipt>;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Receipts {
    #[beserial(len_type(u16))]
    pub transactions: Vec<TransactionOperationReceipt>,
    #[beserial(len_type(u16))]
    pub inherents: Vec<InherentOperationReceipt>,
}

// TODO Implement sparse serialization for Receipts

// impl Receipts {
//     pub fn new() -> Receipts {
//         Receipts(vec![])
//     }
//
//     pub fn len(&self) -> usize {
//         self.0.len()
//     }
//
//     pub fn is_empty(&self) -> bool {
//         self.0.is_empty()
//     }
// }
//
// impl From<Vec<OperationReceipt>> for Receipts {
//     fn from(val: Vec<OperationReceipt>) -> Self {
//         Receipts(val)
//     }
// }
//
// impl IntoIterator for Receipts {
//     type Item = OperationReceipt;
//     type IntoIter = IntoIter<Self::Item>;
//
//     fn into_iter(self) -> Self::IntoIter {
//         self.0.into_iter()
//     }
// }

impl IntoDatabaseValue for Receipts {
    fn database_byte_size(&self) -> usize {
        self.serialized_size()
    }

    fn copy_into_database(&self, mut bytes: &mut [u8]) {
        Serialize::serialize(&self, &mut bytes).unwrap();
    }
}

impl FromDatabaseValue for Receipts {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self>
    where
        Self: Sized,
    {
        let mut cursor = io::Cursor::new(bytes);
        Ok(Deserialize::deserialize(&mut cursor)?)
    }
}
