use std::collections::{btree_map, BTreeMap};

use nimiq_serde::{Deserialize, Serialize};

use crate::key_nibbles::KeyNibbles;

#[derive(Clone, Debug, Eq, Deserialize, PartialEq, Serialize)]
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

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct TrieDiff {
    changes: BTreeMap<KeyNibbles, ValueChange>,
}

impl TrieDiff {
    pub fn add_change(&mut self, key: KeyNibbles, value: ValueChange) {
        // TODO: maybe optimize for the non-existing case
        if let Some(result) = ValueChange::combine(self.changes.remove(&key), Some(value)) {
            assert!(self.changes.insert(key, result).is_none());
        }
    }
    pub fn iter(&self) -> Iter {
        Iter(self.changes.iter())
    }
}

pub struct Iter<'a>(btree_map::Iter<'a, KeyNibbles, ValueChange>);

impl<'a> Iterator for Iter<'a> {
    type Item = (&'a KeyNibbles, &'a ValueChange);
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next()
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl<'a> DoubleEndedIterator for Iter<'a> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.0.next_back()
    }
}
