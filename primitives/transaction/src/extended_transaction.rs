use std::{error, fmt, io};

use nimiq_database_value::{FromDatabaseValue, IntoDatabaseValue};
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_mmr::hash::Hash as MMRHash;
use nimiq_primitives::{coin::Coin, networks::NetworkId, policy::Policy};
use nimiq_serde::{Deserialize, Serialize};

use crate::{inherent::Inherent, ExecutedTransaction, Transaction as BlockchainTransaction};

/// A single struct that stores information that represents any possible transaction (basic
/// transaction or inherent) on the blockchain.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtendedTransaction {
    /// The ID of the network where the transaction happened.
    pub network_id: NetworkId,
    /// The number of the block when the transaction happened.
    pub block_number: u32,
    /// The timestamp of the block when the transaction happened.
    pub block_time: u64,
    /// A struct containing the transaction data.
    pub data: ExtTxData,
}

#[derive(Clone, Debug)]
pub enum IntoTransactionError {
    NoBasicTransactionMapping,
}

impl fmt::Display for IntoTransactionError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use IntoTransactionError::*;
        match self {
            NoBasicTransactionMapping => {
                "no basic transaction mapping for this extended transaction"
            }
        }
        .fmt(f)
    }
}
impl error::Error for IntoTransactionError {}

impl ExtendedTransaction {
    /// Convert a set of inherents and basic transactions (together with a network id, a block
    /// number and a block timestamp) into a vector of extended transactions.
    /// We only want to store punishments and reward inherents, so we ignore the other inherent types.
    pub fn from(
        network_id: NetworkId,
        block_number: u32,
        block_time: u64,
        transactions: Vec<ExecutedTransaction>,
        inherents: Vec<Inherent>,
    ) -> Vec<ExtendedTransaction> {
        let mut ext_txs = vec![];

        for transaction in transactions {
            ext_txs.push(ExtendedTransaction {
                network_id,
                block_number,
                block_time,
                data: ExtTxData::Basic(transaction),
            })
        }

        for inherent in inherents {
            match inherent {
                Inherent::Penalize { .. } | Inherent::Reward { .. } | Inherent::Jail { .. } => {
                    ext_txs.push(ExtendedTransaction {
                        network_id,
                        block_number,
                        block_time,
                        data: ExtTxData::Inherent(inherent),
                    })
                }
                // These special types of inherents do not generate extended transactions
                Inherent::FinalizeBatch | Inherent::FinalizeEpoch => {}
            }
        }

        ext_txs
    }

    /// Convert a set of extended transactions into a vector of inherents and a vector of basic
    /// transactions.
    pub fn to(ext_txs: Vec<ExtendedTransaction>) -> (Vec<ExecutedTransaction>, Vec<Inherent>) {
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

    /// Unwraps the extended transaction and returns a reference to the underlying executed transaction.
    pub fn unwrap_basic(&self) -> &ExecutedTransaction {
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

    /// Returns the hash of the underlying transaction/inherent. For reward inherents we return the
    /// hash of the corresponding reward transaction. This results into an unique hash for the
    /// reward inherents (which wouldn't happen otherwise) and allows front-end to fetch rewards by
    /// their transaction hash.
    pub fn tx_hash(&self) -> Blake2bHash {
        match &self.data {
            ExtTxData::Basic(tx) => tx.hash(),
            ExtTxData::Inherent(v) => match v {
                Inherent::Reward { .. } => self.clone().into_transaction().unwrap().hash(),
                _ => v.hash(),
            },
        }
    }

    /// Tries to convert an extended transaction into a regular transaction. This will work for all
    /// extended transactions that wrap over regular transactions and reward inherents.
    pub fn into_transaction(self) -> Result<ExecutedTransaction, IntoTransactionError> {
        match self.data {
            ExtTxData::Basic(tx) => Ok(tx),
            ExtTxData::Inherent(inherent) => {
                if let Inherent::Reward { target, value } = inherent {
                    let txn = BlockchainTransaction::new_basic(
                        Policy::COINBASE_ADDRESS,
                        target,
                        value,
                        Coin::ZERO,
                        self.block_number,
                        self.network_id,
                    );
                    Ok(ExecutedTransaction::Ok(txn))
                } else {
                    Err(IntoTransactionError::NoBasicTransactionMapping)
                }
            }
        }
    }
}

impl MMRHash<Blake2bHash> for ExtendedTransaction {
    /// Hashes a prefix and an extended transaction into a Blake2bHash. The prefix is necessary
    /// to include it into the History Tree.
    fn hash(&self, prefix: u64) -> Blake2bHash {
        let mut message = prefix.to_be_bytes().to_vec();
        message.append(&mut self.serialize_to_vec());
        message.hash()
    }
}

impl IntoDatabaseValue for ExtendedTransaction {
    fn database_byte_size(&self) -> usize {
        self.serialized_size()
    }

    fn copy_into_database(&self, mut bytes: &mut [u8]) {
        Serialize::serialize_to_writer(&self, &mut bytes).unwrap();
    }
}

impl FromDatabaseValue for ExtendedTransaction {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self>
    where
        Self: Sized,
    {
        Deserialize::deserialize_from_vec(bytes)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
    }
}

/// An enum specifying the type of transaction and containing the necessary data to represent that
/// transaction.
// TODO: The transactions include a lot of unnecessary information (ex: the signature). Don't
//       include all of it here.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ExtTxData {
    /// A basic transaction. It simply contains the transaction as contained in the block.
    Basic(ExecutedTransaction),
    /// An inherent transaction. It simply contains the transaction as contained in the block.
    Inherent(Inherent),
}
