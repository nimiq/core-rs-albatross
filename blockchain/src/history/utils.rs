use std::{borrow::Cow, convert::TryInto};

use nimiq_database::utils::IndexedValue;
use nimiq_database_value::{AsDatabaseBytes, FromDatabaseBytes};
use nimiq_hash::Blake2bHash;
use nimiq_transaction::historic_transaction::HistoricTransaction;

pub type OrderedHash = IndexedValue<EpochBasedIndex, Blake2bHash>;
pub type IndexedTransaction = IndexedValue<u32, StoredHistoricTransaction>;
pub type IndexedHash = IndexedValue<u32, Blake2bHash>;

/// A historic transaction as stored in the `HistoricTransactions` dup table.
///
/// Dup table values are limited to `MAX_DUP_VALUE_SIZE` bytes. Historic transactions that do not
/// fit are stored in a separate regular table, and only a marker is kept in the dup table so that
/// the leaf indices stay contiguous.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoredHistoricTransaction {
    /// The historic transaction itself.
    Inline(HistoricTransaction),
    /// A marker for a historic transaction that is stored in the oversized transactions table.
    Oversized,
}

impl AsDatabaseBytes for StoredHistoricTransaction {
    fn as_key_bytes(&self) -> Cow<'_, [u8]> {
        match self {
            // Same encoding as a plain historic transaction, so existing databases stay valid
            StoredHistoricTransaction::Inline(hist_tx) => hist_tx.as_value_bytes(),
            // A serialized historic transaction is never empty, so the empty value is free to
            // use as the marker
            StoredHistoricTransaction::Oversized => Cow::Borrowed(&[]),
        }
    }
}

impl FromDatabaseBytes for StoredHistoricTransaction {
    fn from_key_bytes(bytes: &[u8]) -> Self
    where
        Self: Sized,
    {
        if bytes.is_empty() {
            StoredHistoricTransaction::Oversized
        } else {
            StoredHistoricTransaction::Inline(HistoricTransaction::from_value_bytes(bytes))
        }
    }
}

/// A wrapper for an u32 and a u32.
/// We use it to store the epoch number and the (leaf) index of a transaction in the epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct EpochBasedIndex {
    pub epoch_number: u32,
    pub index: u32,
}

impl EpochBasedIndex {
    pub fn new(epoch_number: u32, index: u32) -> Self {
        Self {
            epoch_number,
            index,
        }
    }
}

impl AsDatabaseBytes for EpochBasedIndex {
    fn as_key_bytes(&self) -> Cow<'_, [u8]> {
        let bytes = [
            &self.epoch_number.to_be_bytes()[..],
            &self.index.to_be_bytes()[..],
        ]
        .concat();
        Cow::Owned(bytes)
    }

    const FIXED_SIZE: Option<usize> = Some(8);
}

impl FromDatabaseBytes for EpochBasedIndex {
    fn from_key_bytes(bytes: &[u8]) -> Self
    where
        Self: Sized,
    {
        EpochBasedIndex {
            epoch_number: u32::from_be_bytes(bytes[..4].try_into().unwrap()),
            index: u32::from_be_bytes(bytes[4..].try_into().unwrap()),
        }
    }
}
