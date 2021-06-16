use std::cmp::Ordering;
use std::io;

use beserial::{
    Deserialize, DeserializeWithLength, ReadBytesExt, Serialize, SerializeWithLength,
    SerializingError, WriteBytesExt,
};
use nimiq_database::{FromDatabaseValue, IntoDatabaseValue};
use nimiq_hash::{Hash, HashOutput, Hasher, SerializeContent};
use nimiq_keys::Address;

use crate::Account;

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
            Receipt::Transaction { .. } => ReceiptType::Transaction,
            Receipt::Inherent { .. } => ReceiptType::Inherent,
        }
    }
}

impl Serialize for Receipt {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = self.receipt_type().serialize(writer)?;
        match self {
            Receipt::PrunedAccount(pruned_account) => size += pruned_account.serialize(writer)?,
            Receipt::Transaction {
                index,
                sender,
                data,
            } => {
                size += index.serialize(writer)?;
                size += sender.serialize(writer)?;
                size += SerializeWithLength::serialize::<u16, W>(data, writer)?;
            }
            Receipt::Inherent {
                index,
                data,
                pre_transactions,
            } => {
                size += index.serialize(writer)?;
                size += SerializeWithLength::serialize::<u16, W>(data, writer)?;
                size += pre_transactions.serialize(writer)?;
            }
        }
        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size = self.receipt_type().serialized_size();
        match self {
            Receipt::PrunedAccount(pruned_account) => size += pruned_account.serialized_size(),
            Receipt::Transaction {
                index,
                sender,
                data,
            } => {
                size += index.serialized_size();
                size += sender.serialized_size();
                size += SerializeWithLength::serialized_size::<u16>(data);
            }
            Receipt::Inherent {
                index,
                data,
                pre_transactions,
            } => {
                size += index.serialized_size();
                size += SerializeWithLength::serialized_size::<u16>(data);
                size += pre_transactions.serialized_size();
            }
        }
        size
    }
}

impl Deserialize for Receipt {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let ty: ReceiptType = Deserialize::deserialize(reader)?;
        match ty {
            ReceiptType::PrunedAccount => {
                Ok(Receipt::PrunedAccount(Deserialize::deserialize(reader)?))
            }
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
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> {
        Ok(self.serialize(writer)?)
    }
}

#[allow(clippy::derive_hash_xor_eq)] // TODO: Shouldn't be necessary
impl Hash for Receipt {
    fn hash<H: HashOutput>(&self) -> H {
        let h = H::Builder::default();
        self.serialize_content(&mut vec![]).unwrap();
        h.finish()
    }
}

impl Ord for Receipt {
    fn cmp(&self, other: &Receipt) -> Ordering {
        match (self, other) {
            (Receipt::PrunedAccount(a), Receipt::PrunedAccount(b)) => a.cmp(b),
            (
                Receipt::Transaction {
                    index: index_a,
                    sender: sender_a,
                    data: data_a,
                },
                Receipt::Transaction {
                    index: index_b,
                    sender: sender_b,
                    data: data_b,
                },
            ) => {
                // Ensure order is the same as when processing the block.
                sender_a
                    .cmp(sender_b)
                    .reverse()
                    .then_with(|| index_a.cmp(index_b))
                    .then_with(|| data_a.cmp(data_b))
            }
            (
                Receipt::Inherent {
                    index: index_a,
                    data: data_a,
                    pre_transactions: pre_transactions_a,
                },
                Receipt::Inherent {
                    index: index_b,
                    data: data_b,
                    pre_transactions: pre_transactions_b,
                },
            ) => pre_transactions_a
                .cmp(pre_transactions_b)
                .reverse()
                .then_with(|| index_a.cmp(index_b))
                .then_with(|| data_a.cmp(data_b)),
            (a, b) => a.receipt_type().cmp(&b.receipt_type()),
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

#[derive(Clone, Eq, Debug, Serialize, Deserialize)]
pub struct PrunedAccount {
    pub address: Address,
    pub account: Account,
}

impl SerializeContent for PrunedAccount {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> {
        Ok(self.serialize(writer)?)
    }
}

impl Hash for PrunedAccount {
    fn hash<H: HashOutput>(&self) -> H {
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
