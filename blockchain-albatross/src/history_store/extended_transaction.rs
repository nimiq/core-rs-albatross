use crate::history_store::history_tree_hash::HistoryTreeHash;
use beserial::{Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};
use database::{FromDatabaseValue, IntoDatabaseValue};
use hash::Hash;
use mmr::hash::Hash as MMRHash;
use std::io;
use transaction::Transaction as BlockchainTransaction;

#[derive(Clone, Debug)]
pub struct ExtendedTransaction {
    pub block_number: u32,
    pub block_time: u64,
    pub data: ExtendedTransactionData,
}

impl MMRHash<HistoryTreeHash> for &ExtendedTransaction {
    fn hash(&self, prefix: u64) -> HistoryTreeHash {
        let mut message = prefix.to_be_bytes().to_vec();
        message.append(&mut self.serialize_to_vec());
        HistoryTreeHash(message.hash())
    }
}

impl Serialize for ExtendedTransaction {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = 0;
        size += Serialize::serialize(&self.block_number, writer)?;
        size += Serialize::serialize(&self.block_time, writer)?;
        size += Serialize::serialize(&self.data, writer)?;
        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size = 0;
        size += Serialize::serialized_size(&self.block_number);
        size += Serialize::serialized_size(&self.block_time);
        size += Serialize::serialized_size(&self.data);
        size
    }
}

impl Deserialize for ExtendedTransaction {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let block_number: u32 = Deserialize::deserialize(reader)?;
        let block_time: u64 = Deserialize::deserialize(reader)?;
        let data: ExtendedTransactionData = Deserialize::deserialize(reader)?;
        Ok(ExtendedTransaction {
            block_number,
            block_time,
            data,
        })
    }
}

impl IntoDatabaseValue for ExtendedTransaction {
    fn database_byte_size(&self) -> usize {
        self.serialized_size()
    }

    fn copy_into_database(&self, mut bytes: &mut [u8]) {
        Serialize::serialize(&self, &mut bytes).unwrap();
    }
}

impl FromDatabaseValue for ExtendedTransaction {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self>
    where
        Self: Sized,
    {
        let mut cursor = io::Cursor::new(bytes);
        Ok(Deserialize::deserialize(&mut cursor)?)
    }
}

#[derive(Clone, Debug)]
pub enum ExtendedTransactionData {
    Basic(BlockchainTransaction),
    // This the block number and the view number, in that order.
    Fork(u32, u32),
    // This is the view number.
    ViewChange(u32),
}

impl Serialize for ExtendedTransactionData {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        match self {
            ExtendedTransactionData::Basic(tx) => {
                let mut size = 0;
                size += Serialize::serialize(&ExtendedTransactionDataType::Basic, writer)?;
                size += Serialize::serialize(tx, writer)?;
                Ok(size)
            }
            ExtendedTransactionData::Fork(block_nr, view_nr) => {
                let mut size = 0;
                size += Serialize::serialize(&ExtendedTransactionDataType::Fork, writer)?;
                size += Serialize::serialize(block_nr, writer)?;
                size += Serialize::serialize(view_nr, writer)?;
                Ok(size)
            }
            ExtendedTransactionData::ViewChange(view_nr) => {
                let mut size = 0;
                size += Serialize::serialize(&ExtendedTransactionDataType::ViewChange, writer)?;
                size += Serialize::serialize(view_nr, writer)?;
                Ok(size)
            }
        }
    }

    fn serialized_size(&self) -> usize {
        match self {
            ExtendedTransactionData::Basic(tx) => {
                let mut size = 1;
                size += Serialize::serialized_size(tx);
                size
            }
            ExtendedTransactionData::Fork(block_nr, view_nr) => {
                let mut size = 1;
                size += Serialize::serialized_size(block_nr);
                size += Serialize::serialized_size(view_nr);
                size
            }
            ExtendedTransactionData::ViewChange(view_nr) => {
                let mut size = 1;
                size += Serialize::serialized_size(view_nr);
                size
            }
        }
    }
}

impl Deserialize for ExtendedTransactionData {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let ext_tx_data_type: ExtendedTransactionDataType = Deserialize::deserialize(reader)?;
        match ext_tx_data_type {
            ExtendedTransactionDataType::Basic => {
                let tx: BlockchainTransaction = Deserialize::deserialize(reader)?;
                Ok(ExtendedTransactionData::Basic(tx))
            }
            ExtendedTransactionDataType::Fork => {
                let block_nr: u32 = Deserialize::deserialize(reader)?;
                let view_nr: u32 = Deserialize::deserialize(reader)?;
                Ok(ExtendedTransactionData::Fork(block_nr, view_nr))
            }
            ExtendedTransactionDataType::ViewChange => {
                let view_nr: u32 = Deserialize::deserialize(reader)?;
                Ok(ExtendedTransactionData::ViewChange(view_nr))
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum ExtendedTransactionDataType {
    Basic,
    Fork,
    ViewChange,
}
