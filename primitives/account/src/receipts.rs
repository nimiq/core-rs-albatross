use std::io;

use beserial::{
    Deserialize, DeserializeWithLength, ReadBytesExt, Serialize, SerializeWithLength,
    SerializingError, WriteBytesExt,
};
use nimiq_database::{FromDatabaseValue, IntoDatabaseValue};
use nimiq_hash::{Hash, HashOutput, Hasher, SerializeContent};

#[derive(Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum ReceiptType {
    Transaction = 1,
    Inherent = 2,
}

#[derive(Clone, Eq, PartialEq, Debug, Ord, PartialOrd)]
pub enum Receipt {
    Transaction {
        index: u16,
        sender: bool,
        data: Option<Vec<u8>>,
    },
    Inherent {
        index: u16,
        pre_transactions: bool,
        data: Option<Vec<u8>>,
    },
}

impl Receipt {
    pub fn receipt_type(&self) -> ReceiptType {
        match self {
            Receipt::Transaction { .. } => ReceiptType::Transaction,
            Receipt::Inherent { .. } => ReceiptType::Inherent,
        }
    }
}

impl Serialize for Receipt {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = self.receipt_type().serialize(writer)?;
        match self {
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
                size += pre_transactions.serialize(writer)?;
                size += SerializeWithLength::serialize::<u16, W>(data, writer)?;
            }
        }
        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size = self.receipt_type().serialized_size();
        match self {
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
                size += pre_transactions.serialized_size();
                size += SerializeWithLength::serialized_size::<u16>(data);
            }
        }
        size
    }
}

impl Deserialize for Receipt {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let ty: ReceiptType = Deserialize::deserialize(reader)?;
        match ty {
            ReceiptType::Transaction => Ok(Receipt::Transaction {
                index: Deserialize::deserialize(reader)?,
                sender: Deserialize::deserialize(reader)?,
                data: DeserializeWithLength::deserialize::<u16, R>(reader)?,
            }),
            ReceiptType::Inherent => Ok(Receipt::Inherent {
                index: Deserialize::deserialize(reader)?,
                pre_transactions: Deserialize::deserialize(reader)?,
                data: DeserializeWithLength::deserialize::<u16, R>(reader)?,
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

#[derive(Default, Clone, Eq, PartialEq, Debug, Serialize, Deserialize)]
pub struct Receipts {
    #[beserial(len_type(u16))]
    pub receipts: Vec<Receipt>,
}

impl Receipts {
    pub fn new() -> Receipts {
        Receipts { receipts: vec![] }
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
