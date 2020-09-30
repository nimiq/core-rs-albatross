use crate::history_store::HistoryTreeHash;
use beserial::{Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};
use database::{FromDatabaseValue, IntoDatabaseValue};
use hash::Hash;
use mmr::hash::Hash as MMRHash;
use std::io;
use transaction::Transaction as BlockchainTransaction;

/// A single struct that stores information that represents any possible transaction (account
/// transaction, fork, view change) on the blockchain.
#[derive(Clone, Debug)]
pub struct ExtendedTransaction {
    // The number of the block when the transaction happened.
    pub block_number: u32,
    // The timestamp of the block when the transaction happened.
    pub block_time: u64,
    // A struct containing the transaction data.
    pub data: ExtendedTransactionData,
}

impl MMRHash<HistoryTreeHash> for &ExtendedTransaction {
    /// Hashes a prefix and an extended transaction into a HistoryTreeHash. The prefix is necessary
    /// to include it into the History Tree.
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

/// An enum specifying the type of transaction and containing the necessary data to represent that
/// transaction.
#[derive(Clone, Debug)]
pub enum ExtendedTransactionData {
    // A basic transaction. It simply contains the transaction as contained in the block.
    // TODO: The transaction includes a lot of unnecessary information (ex: the signature). Don't
    //       include all of it here.
    Basic(BlockchainTransaction),
    // A fork transaction. It only specifies the validator slot that got slashed as result of the
    // fork. The proof is omitted.
    Fork(u16),
    // A view change transaction. It only specifies the validator slot(s) that got slashed as result
    // of the view change. The proof is omitted.
    ViewChange(Vec<u16>),
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
            ExtendedTransactionData::Fork(slot) => {
                let mut size = 0;
                size += Serialize::serialize(&ExtendedTransactionDataType::Fork, writer)?;
                size += Serialize::serialize(slot, writer)?;
                Ok(size)
            }
            ExtendedTransactionData::ViewChange(slots) => {
                let mut size = 0;
                size += Serialize::serialize(&ExtendedTransactionDataType::ViewChange, writer)?;
                size += Serialize::serialize(&(slots.len() as u16), writer)?;
                for slot in slots {
                    size += Serialize::serialize(slot, writer)?;
                }
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
            ExtendedTransactionData::Fork(_) => 1 + 4,
            ExtendedTransactionData::ViewChange(slots) => {
                let mut size = 1 + 4;
                size += slots.len() * 2;
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
                let slot: u16 = Deserialize::deserialize(reader)?;
                Ok(ExtendedTransactionData::Fork(slot))
            }
            ExtendedTransactionDataType::ViewChange => {
                let mut slots = vec![];
                let len = Deserialize::deserialize(reader)?;
                for _i in 0..len {
                    slots.push(Deserialize::deserialize(reader)?);
                }
                Ok(ExtendedTransactionData::ViewChange(slots))
            }
        }
    }
}

/// Just a convenience enum to help with the serialization/deserialization functions.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum ExtendedTransactionDataType {
    Basic,
    Fork,
    ViewChange,
}
