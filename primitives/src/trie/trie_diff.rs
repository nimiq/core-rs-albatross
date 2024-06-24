use std::{collections::BTreeMap, io};

use nimiq_database_value::{FromDatabaseValue, IntoDatabaseValue};
use nimiq_serde::{Deserialize, Serialize};

use crate::key_nibbles::KeyNibbles;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[repr(u8)]
pub enum ValueChange {
    Insert(Vec<u8>),
    Update(Vec<u8>, Vec<u8>),
    Delete(Vec<u8>),
}

impl ValueChange {
    pub fn invert(self) -> ValueChange {
        use ValueChange::*;
        match self {
            Insert(n) => Delete(n),
            Update(o, n) => Update(n, o),
            Delete(o) => Insert(o),
        }
    }
    fn into_old_and_new(self) -> (Option<Vec<u8>>, Option<Vec<u8>>) {
        use ValueChange::*;
        match self {
            Insert(n) => (None, Some(n)),
            Update(o, n) => (Some(o), Some(n)),
            Delete(o) => (Some(o), None),
        }
    }
    fn from_old_and_new(old: Option<Vec<u8>>, new: Option<Vec<u8>>) -> Option<ValueChange> {
        use ValueChange::*;
        match (old, new) {
            (None, None) => None,
            (None, Some(n)) => Some(Insert(n)),
            (Some(o), Some(n)) => {
                if o != n {
                    Some(Update(o, n))
                } else {
                    None
                }
            }
            (Some(o), None) => Some(Delete(o)),
        }
    }
    pub fn combine(left: Option<ValueChange>, right: Option<ValueChange>) -> Option<ValueChange> {
        match (left, right) {
            (None, None) => None,
            (None, Some(right)) => Some(right),
            (Some(left), None) => Some(left),
            (Some(left), Some(right)) => {
                let left = left.into_old_and_new();
                let right = right.into_old_and_new();
                assert!(left.1 == right.0, "inconsistent state");
                ValueChange::from_old_and_new(left.0, right.1)
            }
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct TrieDiffBuilder {
    pub changes: BTreeMap<KeyNibbles, ValueChange>,
}

impl TrieDiffBuilder {
    pub fn add_change(&mut self, key: KeyNibbles, value: ValueChange) {
        // TODO: maybe optimize for the non-existing case
        if let Some(result) = ValueChange::combine(self.changes.remove(&key), Some(value)) {
            assert!(self.changes.insert(key, result).is_none());
        }
    }
    pub fn into_backward_diff(self) -> TrieDiff {
        TrieDiff(
            self.changes
                .into_iter()
                .map(|(k, v)| (k, v.into_old_and_new().0))
                .collect(),
        )
    }
    pub fn into_forward_diff(self) -> TrieDiff {
        TrieDiff(
            self.changes
                .into_iter()
                .map(|(k, v)| (k, v.into_old_and_new().1))
                .collect(),
        )
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TrieDiff(pub BTreeMap<KeyNibbles, Option<Vec<u8>>>);

impl IntoDatabaseValue for TrieDiff {
    fn database_byte_size(&self) -> usize {
        self.serialized_size()
    }

    fn copy_into_database(&self, mut bytes: &mut [u8]) {
        Serialize::serialize(&self, &mut bytes).unwrap();
    }
}

impl FromDatabaseValue for TrieDiff {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self>
    where
        Self: Sized,
    {
        Self::deserialize_from_vec(bytes).map_err(|e| io::Error::new(io::ErrorKind::Other, e))
    }
}

/// A diff on an incomplete trie.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[repr(u8)]
pub enum RevertDiffValue {
    /// Puts a value into the known part of the trie.
    Put(Vec<u8>),
    /// Removes a value from either the known or the unknown part of the trie.
    Remove,
    /// Updates a value in the unknown part of the trie.
    UpdateStump,
}

impl RevertDiffValue {
    /// Create an incomplete diff value for a value from the known part of the trie.
    pub fn known_value(old_value: Option<Vec<u8>>) -> Self {
        match old_value {
            Some(v) => RevertDiffValue::Put(v),
            None => RevertDiffValue::Remove,
        }
    }

    /// Create an incomplete diff value for the unknown part of the trie.
    pub fn unknown_value(new_stump: bool) -> Self {
        match new_stump {
            true => RevertDiffValue::Remove,
            false => RevertDiffValue::UpdateStump,
        }
    }
}

/// A diff on an incomplete trie.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RevertTrieDiff(pub BTreeMap<KeyNibbles, RevertDiffValue>);

impl IntoDatabaseValue for RevertTrieDiff {
    fn database_byte_size(&self) -> usize {
        self.serialized_size()
    }

    fn copy_into_database(&self, mut bytes: &mut [u8]) {
        Serialize::serialize(&self, &mut bytes).unwrap();
    }
}

impl FromDatabaseValue for RevertTrieDiff {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self>
    where
        Self: Sized,
    {
        Self::deserialize_from_vec(bytes).map_err(|e| io::Error::new(io::ErrorKind::Other, e))
    }
}
