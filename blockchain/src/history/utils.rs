use std::{borrow::Cow, convert::TryInto};

use nimiq_database::utils::IndexedValue;
use nimiq_database_value::{AsDatabaseBytes, FromDatabaseBytes};
use nimiq_hash::Blake2bHash;
use nimiq_transaction::historic_transaction::HistoricTransaction;

pub type OrderedHash = IndexedValue<EpochBasedIndex, Blake2bHash>;
pub type IndexedTransaction = IndexedValue<u32, HistoricTransaction>;
pub type IndexedHash = IndexedValue<u32, Blake2bHash>;

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
    fn as_key_bytes(&self) -> Cow<[u8]> {
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
