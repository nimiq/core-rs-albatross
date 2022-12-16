use std::fmt::Debug;
use std::io;

use beserial::{Deserialize, Serialize};
use nimiq_database::{FromDatabaseValue, IntoDatabaseValue};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AccountReceipt(#[beserial(len_type(u16))] Vec<u8>);

impl From<Vec<u8>> for AccountReceipt {
    fn from(val: Vec<u8>) -> Self {
        AccountReceipt(val)
    }
}

impl<T: Serialize> From<T> for AccountReceipt {
    fn from(val: T) -> Self {
        AccountReceipt(val.serialize_to_vec())
    }
}

impl<T: Deserialize> Into<T> for AccountReceipt {
    fn into(self) -> T {
        T::deserialize(&mut self.0[..])
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
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
    pub transactions: Vec<TransactionOperationReceipt>,
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
