use std::{
    borrow::Cow,
    error, fmt,
    io::{self, Write},
    ops::{Deref, Range},
};

use nimiq_database_value::{AsDatabaseBytes, FromDatabaseValue, IntoDatabaseValue};
use nimiq_hash::{Blake2bHash, Blake2bHasher, Hash, Hasher};
use nimiq_hash_derive::SerializeContent;
use nimiq_keys::Address;
use nimiq_mmr::hash::Hash as MMRHash;
use nimiq_primitives::{coin::Coin, networks::NetworkId};
use nimiq_serde::{Deserialize, Serialize};

use crate::{inherent::Inherent, EquivocationLocator, ExecutedTransaction};

/// The raw transaction hash is a type wrapper.
/// This corresponds to the hash of the transaction without the execution result.
/// This hash is the external facing hash.
#[derive(Clone, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct RawTransactionHash(Blake2bHash);

impl From<Blake2bHash> for RawTransactionHash {
    fn from(hash: Blake2bHash) -> Self {
        RawTransactionHash(hash)
    }
}
impl From<RawTransactionHash> for Blake2bHash {
    fn from(hash: RawTransactionHash) -> Self {
        hash.0
    }
}
impl Deref for RawTransactionHash {
    type Target = Blake2bHash;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl AsDatabaseBytes for RawTransactionHash {
    fn as_database_bytes(&self) -> Cow<[u8]> {
        Cow::Owned(self.deref().0.to_vec())
    }
}
impl FromDatabaseValue for RawTransactionHash {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self>
    where
        Self: Sized,
    {
        Ok(RawTransactionHash(bytes[4..].into()))
    }
}

/// The executed transaction hash is a type wrapper.
/// This corresponds to the hash of the transaction with the execution result.
/// This hash is used in the history store so that we have a record of the transaction
/// and its execution result. This is an internal to it.
#[derive(Clone, Debug)]
struct ExecutedTransactionHash(Blake2bHash);

impl From<Blake2bHash> for ExecutedTransactionHash {
    fn from(hash: Blake2bHash) -> Self {
        ExecutedTransactionHash(hash)
    }
}
impl Deref for ExecutedTransactionHash {
    type Target = Blake2bHash;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl AsDatabaseBytes for ExecutedTransactionHash {
    fn as_database_bytes(&self) -> Cow<[u8]> {
        Cow::Owned(self.deref().0.to_vec())
    }
}
impl FromDatabaseValue for ExecutedTransactionHash {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self>
    where
        Self: Sized,
    {
        Ok(ExecutedTransactionHash(bytes[4..].into()))
    }
}

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

/// Why a historic transaction cannot be represented as a basic transaction.
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
        equivocation_locator: Vec<EquivocationLocator>,
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

        for locator in equivocation_locator {
            hist_txs.push(HistoricTransaction {
                network_id,
                block_number,
                block_time,
                data: HistoricTransactionData::Equivocation(EquivocationEvent { locator }),
            });
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
                Inherent::Penalize { slot } => hist_txs.push(HistoricTransaction {
                    network_id,
                    block_number,
                    block_time,
                    data: HistoricTransactionData::Penalize(PenalizeEvent {
                        validator_address: slot.validator_address,
                        slot: slot.slot,
                        offense_event_block: slot.offense_event_block,
                    }),
                }),
                Inherent::Jail {
                    jailed_validator,
                    new_epoch_slot_range,
                } => hist_txs.push(HistoricTransaction {
                    network_id,
                    block_number,
                    block_time,
                    data: HistoricTransactionData::Jail(JailEvent {
                        validator_address: jailed_validator.validator_address,
                        slots: jailed_validator.slots,
                        offense_event_block: jailed_validator.offense_event_block,
                        new_epoch_slot_range,
                    }),
                }),
                Inherent::FinalizeBatch => {}
                Inherent::FinalizeEpoch => {}
            }
        }

        hist_txs
    }

    pub fn count(
        num_transactions: usize,
        inherents: &[Inherent],
        equivocation_locator: Vec<EquivocationLocator>,
    ) -> usize {
        num_transactions
            + inherents
                .iter()
                .filter_map(|inherent| match inherent {
                    Inherent::Reward { .. } => Some(()),
                    Inherent::Penalize { .. } => Some(()),
                    Inherent::Jail { .. } => Some(()),
                    Inherent::FinalizeBatch => None,
                    Inherent::FinalizeEpoch => None,
                })
                .count()
            + equivocation_locator.len()
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

    /// Returns the hash based on the underlying transaction/event.
    /// In case of a basic historic transaction, it returns the executed transaction hash.
    fn executed_tx_hash(&self) -> ExecutedTransactionHash {
        match &self.data {
            HistoricTransactionData::Basic(tx) => tx.hash::<Blake2bHash>().into(),
            _ => {
                let block_height = if matches!(self.data, HistoricTransactionData::Equivocation(_))
                {
                    // Equivocations are not distinguished by block height.
                    0
                } else {
                    self.block_number
                };
                let mut hasher = Blake2bHasher::new();

                // The following ensures a separate hashing-namespace for
                // inherents, by preventing self.data from serializing to a
                // valid transaction (accidentally or on purpose), by following
                // the serialization of a transaction until an impossible byte
                // (sender type).
                hasher.write_all(b"\x00\x00").unwrap(); // recipient data len
                {} // recipient data
                hasher
                    .write_all(b"\x00\x00\x00\x00\x00\x00\x00\x00")
                    .unwrap(); // sender
                hasher
                    .write_all(b"\x00\x00\x00\x00\x00\x00\x00\x00")
                    .unwrap(); // sender
                hasher.write_all(b"\x00\x00\x00\x00").unwrap(); // sender
                hasher.write_all(b"\xff").unwrap(); // sender_type (HistoricTransaction = 0xff)

                // Hash the actual data.
                self.network_id.serialize_to_writer(&mut hasher).unwrap();
                block_height.serialize_to_writer(&mut hasher).unwrap();
                self.data.serialize_to_writer(&mut hasher).unwrap();
                hasher.finish().into()
            }
        }
    }

    /// Returns the hash based on the underlying transaction/event.
    /// In case of a basic historic transaction, it returns the hash of the inner transaction,
    /// without the execution result.
    pub fn tx_hash(&self) -> RawTransactionHash {
        match &self.data {
            HistoricTransactionData::Basic(tx) => tx.raw_tx_hash(),
            _ => self.executed_tx_hash().hash::<Blake2bHash>().into(),
        }
    }
}

impl MMRHash<Blake2bHash> for HistoricTransaction {
    /// Hashes a prefix and an historic transaction into a Blake2bHash. The prefix is necessary
    /// to include it into the History Tree.
    /// This returns the executed transaction hash prefixed with 'prefix' number of leaves.
    fn hash(&self, prefix: u64) -> Blake2bHash {
        let mut hasher = Blake2bHasher::new();
        hasher.write_all(&prefix.to_be_bytes()).unwrap();
        self.serialize_to_writer(&mut hasher).unwrap();
        hasher.finish()
    }
}

impl AsDatabaseBytes for HistoricTransaction {
    fn as_database_bytes(&self) -> Cow<[u8]> {
        Cow::Owned(Serialize::serialize_to_vec(&self))
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
        Self::deserialize_from_vec(bytes).map_err(|e| io::Error::new(io::ErrorKind::Other, e))
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
    /// A reward for an active validator.
    Reward(RewardEvent),
    /// A penalty for an inactive or non-responsive validator.
    Penalize(PenalizeEvent),
    /// A larger penalty for a misbehaving validator.
    Jail(JailEvent),
    /// A record that an equivocation proof was presented.
    Equivocation(EquivocationEvent),
}

/// An equivocation proof was presented. Only the location of it is stored.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EquivocationEvent {
    /// The locator of the equivocation that happened.
    pub locator: EquivocationLocator,
}

/// A reward was paid out to an active validator.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RewardEvent {
    /// The validator address of the rewarded validator.
    pub validator_address: Address,
    /// The address the reward was paid out to.
    pub reward_address: Address,
    /// The reward amount.
    pub value: Coin,
}

/// An inactive or non-responsive validator was punished.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PenalizeEvent {
    /// The validator address of the offending validator.
    pub validator_address: Address,
    /// The slot of the validator that was punished.
    pub slot: u16,
    /// The block height at which the offense occurred.
    pub offense_event_block: u32,
}

/// A misbehaving validator was punished.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct JailEvent {
    /// The validator address of the offending validator.
    pub validator_address: Address,
    /// The slots of the offending validator.
    pub slots: Range<u16>,
    /// The block height at which the offense occurred.
    pub offense_event_block: u32,
    /// Slot range of the validator in the next epoch.
    pub new_epoch_slot_range: Option<Range<u16>>,
}
