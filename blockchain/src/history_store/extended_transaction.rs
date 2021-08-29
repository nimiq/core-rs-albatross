use std::io;

use nimiq_mmr::hash::Hash as MMRHash;

use beserial::{Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};
use nimiq_account::{Inherent, InherentType};
use nimiq_database::{FromDatabaseValue, IntoDatabaseValue};
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_transaction::Transaction as BlockchainTransaction;

use crate::history_store::history_tree_hash::HistoryTreeHash;

/// A single struct that stores information that represents any possible transaction (basic
/// transaction or inherent) on the blockchain.
#[derive(Clone, Debug)]
pub struct ExtendedTransaction {
    // The number of the block when the transaction happened.
    pub block_number: u32,
    // The timestamp of the block when the transaction happened.
    pub block_time: u64,
    // A struct containing the transaction data.
    pub data: ExtTxData,
}

impl ExtendedTransaction {
    /// Convert a set of inherents and basic transactions (together with a block number and a block
    /// timestamp) into a vector of extended transactions.
    /// We only want to store slash inherents, so we ignore the other inherent types. TODO
    pub fn from(
        block_number: u32,
        block_time: u64,
        transactions: Vec<BlockchainTransaction>,
        inherents: Vec<Inherent>,
    ) -> Vec<ExtendedTransaction> {
        let mut ext_txs = vec![];

        for transaction in transactions {
            ext_txs.push(ExtendedTransaction {
                block_number,
                block_time,
                data: ExtTxData::Basic(transaction),
            })
        }

        for inherent in inherents {
            if inherent.ty == InherentType::Slash || inherent.ty == InherentType::Reward {
                ext_txs.push(ExtendedTransaction {
                    block_number,
                    block_time,
                    data: ExtTxData::Inherent(inherent),
                })
            }
        }

        ext_txs
    }

    /// Convert a set of extended transactions into a vector of inherents and a vector of basic
    /// transactions.
    pub fn to(ext_txs: Vec<ExtendedTransaction>) -> (Vec<BlockchainTransaction>, Vec<Inherent>) {
        let mut transactions = vec![];
        let mut inherents = vec![];

        for ext_tx in ext_txs {
            match ext_tx.data {
                ExtTxData::Basic(tx) => transactions.push(tx),
                ExtTxData::Inherent(tx) => inherents.push(tx),
            }
        }

        (transactions, inherents)
    }

    /// Checks if the extended transaction is an inherent.
    pub fn is_inherent(&self) -> bool {
        match self.data {
            ExtTxData::Basic(_) => false,
            ExtTxData::Inherent(_) => true,
        }
    }

    /// Unwraps the extended transaction and returns a reference to the underlying basic transaction.
    pub fn unwrap_basic(&self) -> &BlockchainTransaction {
        if let ExtTxData::Basic(ref tx) = self.data {
            tx
        } else {
            unreachable!()
        }
    }

    /// Unwraps the extended transaction and returns a reference to the underlying inherent.
    pub fn unwrap_inherent(&self) -> &Inherent {
        if let ExtTxData::Inherent(ref tx) = self.data {
            tx
        } else {
            unreachable!()
        }
    }

    /// Returns the hash of the underlying transaction.
    pub fn tx_hash(&self) -> Blake2bHash {
        match &self.data {
            ExtTxData::Basic(tx) => tx.hash(),
            ExtTxData::Inherent(tx) => tx.hash(),
        }
    }
}

impl MMRHash<HistoryTreeHash> for ExtendedTransaction {
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
        let data: ExtTxData = Deserialize::deserialize(reader)?;
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
// TODO: The transactions include a lot of unnecessary information (ex: the signature). Don't
//       include all of it here.
#[derive(Clone, Debug)]
pub enum ExtTxData {
    // A basic transaction. It simply contains the transaction as contained in the block.
    Basic(BlockchainTransaction),
    // An inherent transaction. It simply contains the transaction as contained in the block.
    Inherent(Inherent),
}

impl Serialize for ExtTxData {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        match self {
            ExtTxData::Basic(tx) => {
                let mut size = 0;
                size += Serialize::serialize(&ExtendedTransactionDataType::Basic, writer)?;
                size += Serialize::serialize(tx, writer)?;
                Ok(size)
            }
            ExtTxData::Inherent(tx) => {
                let mut size = 0;
                size += Serialize::serialize(&ExtendedTransactionDataType::Inherent, writer)?;
                size += Serialize::serialize(tx, writer)?;
                Ok(size)
            }
        }
    }

    fn serialized_size(&self) -> usize {
        match self {
            ExtTxData::Basic(tx) => {
                let mut size = 1;
                size += Serialize::serialized_size(tx);
                size
            }
            ExtTxData::Inherent(tx) => {
                let mut size = 1;
                size += Serialize::serialized_size(tx);
                size
            }
        }
    }
}

impl Deserialize for ExtTxData {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let ext_tx_data_type: ExtendedTransactionDataType = Deserialize::deserialize(reader)?;
        match ext_tx_data_type {
            ExtendedTransactionDataType::Basic => {
                let tx = Deserialize::deserialize(reader)?;
                Ok(ExtTxData::Basic(tx))
            }
            ExtendedTransactionDataType::Inherent => {
                let tx = Deserialize::deserialize(reader)?;
                Ok(ExtTxData::Inherent(tx))
            }
        }
    }
}

/// Just a convenience enum to help with the serialization/deserialization functions.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum ExtendedTransactionDataType {
    Basic,
    Inherent,
}
