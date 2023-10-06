use std::{error, fmt, io};

use nimiq_database_value::{FromDatabaseValue, IntoDatabaseValue};
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_hash_derive::SerializeContent;
use nimiq_keys::Address;
use nimiq_mmr::hash::Hash as MMRHash;
use nimiq_primitives::{coin::Coin, networks::NetworkId, policy::Policy};
use nimiq_serde::{Deserialize, Serialize};

use crate::{
    inherent::Inherent, EquivocationLocator, ExecutedTransaction,
    Transaction as BlockchainTransaction,
};

/// A single struct that stores information that represents any possible transaction (basic
/// transaction or inherent) on the blockchain.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoricTransaction {
    /// The ID of the network where the transaction happened.
    pub network_id: NetworkId,
    /// The number of the block when the transaction happened.
    pub block_number: u32,
    /// The timestamp of the block when the transaction happened.
    pub block_time: u64,
    /// A struct containing the transaction data.
    pub data: HistoricTransactionData,
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
                "no basic transaction mapping for this historic transaction"
            }
        }
        .fmt(f)
    }
}
impl error::Error for IntoTransactionError {}

impl HistoricTransaction {
    /// Convert a set of inherents and basic transactions (together with a network id, a block
    /// number and a block timestamp) into a vector of historic transactions.
    /// We only want to store punishments and reward inherents, so we ignore the other inherent types.
    pub fn from(
        network_id: NetworkId,
        block_number: u32,
        block_time: u64,
        transactions: Vec<ExecutedTransaction>,
        inherents: Vec<Inherent>,
    ) -> Vec<HistoricTransaction> {
        let mut hist_txs = vec![];

        for transaction in transactions {
            hist_txs.push(HistoricTransaction {
                network_id,
                block_number,
                block_time,
                data: HistoricTransactionData::Basic(transaction),
            })
        }

        for inherent in inherents {
            match inherent {
                Inherent::Reward {
                    validator_address,
                    target,
                    value,
                } => {
                    hist_txs.push(HistoricTransaction {
                        network_id,
                        block_number,
                        block_time,
                        data: HistoricTransactionData::Reward(RewardEvent {
                            validator_address,
                            reward_address: target,
                            value,
                        }),
                    });
                }
                // These special types of inherents do not generate extended transactions
                Inherent::FinalizeBatch | Inherent::FinalizeEpoch => {}
            }
        }

        hist_txs
    }

    /// Checks if the historic transaction is an inherent.
    pub fn is_not_basic(&self) -> bool {
        !matches!(self.data, HistoricTransactionData::Basic(_))
    }

    /// Unwraps the historic transaction and returns a reference to the underlying executed transaction.
    pub fn unwrap_basic(&self) -> &ExecutedTransaction {
        if let HistoricTransactionData::Basic(tx) = &self.data {
            tx
        } else {
            unreachable!()
        }
    }

    /// Unwraps the historic transaction and returns a reference to the underlying reward event.
    pub fn unwrap_reward(&self) -> &RewardEvent {
        if let HistoricTransactionData::Reward(ev) = &self.data {
            ev
        } else {
            unreachable!()
        }
    }

    /*
    /// Unwraps the historic transaction and returns a reference to the underlying inherent.
    pub fn unwrap_inherent(&self) -> &Inherent {
        if let HistoricTransactionData::Inherent(ref tx) = self.data {
            tx
        } else {
            unreachable!()
        }
    }
    */

    /// Returns the hash of the underlying transaction/inherent. For reward inherents we return the
    /// hash of the corresponding reward transaction. This results into an unique hash for the
    /// reward inherents (which wouldn't happen otherwise) and allows front-end to fetch rewards by
    /// their transaction hash.
    pub fn tx_hash(&self) -> Blake2bHash {
        match &self.data {
            HistoricTransactionData::Basic(tx) => tx.hash(),
            // TODO: check for freedom of hash prefixes
            _ => self.data.hash(),
        }
    }

    /// Tries to convert an historic transaction into a regular transaction. This will work for all
    /// historic transactions that wrap over regular transactions and reward inherents.
    pub fn into_transaction(self) -> Result<ExecutedTransaction, IntoTransactionError> {
        match self.data {
            HistoricTransactionData::Basic(tx) => Ok(tx),
            HistoricTransactionData::Reward(ev) => {
                Ok(ExecutedTransaction::Ok(BlockchainTransaction::new_basic(
                    Policy::COINBASE_ADDRESS,
                    ev.reward_address,
                    ev.value,
                    Coin::ZERO,
                    self.block_number,
                    self.network_id,
                )))
            }
            HistoricTransactionData::Equivocation(_) => {
                Err(IntoTransactionError::NoBasicTransactionMapping)
            }
        }
    }
}

impl MMRHash<Blake2bHash> for HistoricTransaction {
    /// Hashes a prefix and an historic transaction into a Blake2bHash. The prefix is necessary
    /// to include it into the History Tree.
    fn hash(&self, prefix: u64) -> Blake2bHash {
        let mut message = prefix.to_be_bytes().to_vec();
        message.append(&mut self.serialize_to_vec());
        message.hash()
    }
}

impl IntoDatabaseValue for HistoricTransaction {
    fn database_byte_size(&self) -> usize {
        self.serialized_size()
    }

    fn copy_into_database(&self, mut bytes: &mut [u8]) {
        Serialize::serialize_to_writer(&self, &mut bytes).unwrap();
    }
}

impl FromDatabaseValue for HistoricTransaction {
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
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, SerializeContent)]
pub enum HistoricTransactionData {
    /// A basic transaction. It simply contains the transaction as contained in the block.
    Basic(ExecutedTransaction),
    Reward(RewardEvent),
    Equivocation(EquivocationEvent),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EquivocationEvent {
    pub locator: EquivocationLocator,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RewardEvent {
    pub validator_address: Address,
    pub reward_address: Address,
    pub value: Coin,
}
