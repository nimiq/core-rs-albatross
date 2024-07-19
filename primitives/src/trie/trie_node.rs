use std::{mem, ops::RangeFrom, slice};

use byteorder::WriteBytesExt;
use log::error;
use nimiq_database_value_derive::DbSerializable;
use nimiq_hash::{Blake2bHash, Hash, HashOutput, Hasher};
use nimiq_serde::Serialize;

use crate::{key_nibbles::KeyNibbles, trie::error::MerkleRadixTrieError};

/// A struct representing a node in the Merkle Radix Trie. It can be either a branch node, which has
/// only references to its children, a leaf node, which contains a value or a hybrid node, which has
/// both children and a value. A branch/hybrid node can have up to 16 children, since we represent
/// the keys in hexadecimal form, each child represents a different hexadecimal character.
#[derive(Clone, Debug, DbSerializable)]
pub struct TrieNode {
    // This is not serialized.
    pub key: KeyNibbles,
    // The root data is not included when the `TrieNode` is hashed.
    pub root_data: Option<RootData>,
    pub value: Option<Vec<u8>>,
    pub children: [Option<TrieNodeChild>; 16],
}

#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde-derive", derive(serde::Serialize, serde::Deserialize))]
pub struct RootData {
    pub incomplete: Option<RangeFrom<KeyNibbles>>,
    pub num_branches: u64,
    pub num_hybrids: u64,
    pub num_leaves: u64,
}

/// A struct representing the child of a node. It just contains the child's suffix (the part of the
/// child's key that is different from its parent) and its hash.
#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Debug)]
#[cfg_attr(feature = "serde-derive", derive(serde::Serialize, serde::Deserialize))]
pub struct TrieNodeChild {
    /// The suffix of this child.
    pub suffix: KeyNibbles,
    /// An all-zero hash (the `Default::default()` value) marks an uncomputed child hash.
    pub hash: Blake2bHash,
}

impl TrieNodeChild {
    pub fn is_stump(
        &self,
        parent_key: &KeyNibbles,
        missing_range: &Option<RangeFrom<KeyNibbles>>,
    ) -> bool {
        missing_range
            .as_ref()
            .map(|range| range.contains(&(parent_key + &self.suffix)))
            .unwrap_or(false)
    }

    pub fn has_hash(&self) -> bool {
        self.hash != Default::default()
    }

    pub fn key(
        &self,
        parent_key: &KeyNibbles,
        missing_range: &Option<RangeFrom<KeyNibbles>>,
    ) -> Result<KeyNibbles, MerkleRadixTrieError> {
        if self.is_stump(parent_key, missing_range) {
            return Err(MerkleRadixTrieError::ChildIsStump);
        }
        Ok(parent_key + &self.suffix)
    }
}

pub const NO_CHILDREN: [Option<TrieNodeChild>; 16] = [
    None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None,
];

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum TrieNodeKind {
    Root,
    Branch,
    Hybrid,
    Leaf,
}

impl TrieNode {
    /// Creates a new leaf node.
    pub fn new_leaf(key: KeyNibbles, value: Vec<u8>) -> Self {
        TrieNode {
            key,
            root_data: None,
            value: Some(value),
            children: NO_CHILDREN,
        }
    }

    /// Creates an empty root node
    pub fn new_root() -> Self {
        Self::new_root_impl(false)
    }

    /// Creates an incomplete root node
    pub fn new_root_incomplete() -> Self {
        Self::new_root_impl(true)
    }

    fn new_root_impl(incomplete: bool) -> Self {
        let children = NO_CHILDREN;
        TrieNode {
            key: KeyNibbles::ROOT,
            root_data: Some(RootData {
                incomplete: if incomplete {
                    Some(KeyNibbles::ROOT..)
                } else {
                    None
                },
                num_branches: 0,
                num_hybrids: 0,
                num_leaves: 0,
            }),
            value: None,
            children,
        }
    }

    /// Creates a new empty branch node.
    pub fn new_empty(key: KeyNibbles) -> Self {
        TrieNode {
            key,
            root_data: None,
            value: None,
            children: NO_CHILDREN,
        }
    }

    pub fn is_empty(&self) -> bool {
        !self.is_root() && self.value.is_none() && !self.has_children()
    }

    pub fn is_root(&self) -> bool {
        self.root_data.is_some()
    }

    pub fn is_hybrid(&self) -> bool {
        !self.is_root() && self.value.is_some() && self.has_children()
    }

    pub fn has_children(&self) -> bool {
        self.iter_children().count() != 0
    }

    pub fn kind(&self) -> Option<TrieNodeKind> {
        Some(
            match (self.is_root(), self.has_children(), self.value.is_some()) {
                (false, false, false) => return None,
                (false, false, true) => TrieNodeKind::Leaf,
                (false, true, false) => TrieNodeKind::Branch,
                (false, true, true) => TrieNodeKind::Hybrid,
                (true, _, _) => TrieNodeKind::Root,
            },
        )
    }

    /// Returns the child index of the given prefix in the current node. If the current node has the
    /// key "31f6d" and the given child key is "31f6d925ca" (both are in hexadecimal) then the child
    /// index is 9.
    pub fn child_index(&self, child_prefix: &KeyNibbles) -> Result<usize, MerkleRadixTrieError> {
        if !self.key.is_prefix_of(child_prefix) {
            error!(
                "Child's prefix {} is not a prefix of the node with key {}!",
                child_prefix, self.key,
            );
            return Err(MerkleRadixTrieError::WrongPrefix);
        }

        // Key length has to be smaller or equal to the child prefix length, so this will only panic
        // when `child_prefix` has the same length as `self.key()`.
        // PITODO: return error instead of unwrapping
        Ok(child_prefix.get(self.key.len()).unwrap())
    }

    /// Returns the current node's child with the given prefix.
    pub fn child(&self, child_prefix: &KeyNibbles) -> Result<&TrieNodeChild, MerkleRadixTrieError> {
        if let Some(child) = &self.children[self.child_index(child_prefix)?] {
            return Ok(child);
        }
        Err(MerkleRadixTrieError::ChildDoesNotExist)
    }

    /// Returns the current node's child with the given prefix.
    pub fn child_mut(
        &mut self,
        child_prefix: &KeyNibbles,
    ) -> Result<&TrieNodeChild, MerkleRadixTrieError> {
        if let Some(child) = &mut self.children[self.child_index(child_prefix)?] {
            return Ok(child);
        }
        Err(MerkleRadixTrieError::ChildDoesNotExist)
    }

    pub fn child_key(
        &self,
        child_prefix: &KeyNibbles,
        missing_range: &Option<RangeFrom<KeyNibbles>>,
    ) -> Result<KeyNibbles, MerkleRadixTrieError> {
        self.child(child_prefix)?.key(&self.key, missing_range)
    }

    /// Sets the current node's child with the given prefix.
    pub fn put_child(
        &mut self,
        child_key: &KeyNibbles,
        child_hash: Blake2bHash,
    ) -> Result<(), MerkleRadixTrieError> {
        let idx = self.child_index(child_key)?;
        let suffix = child_key.suffix(self.key.len() as u8);
        self.children[idx] = Some(TrieNodeChild {
            suffix,
            hash: child_hash,
        });
        Ok(())
    }

    pub fn put_child_no_hash(
        &mut self,
        child_key: &KeyNibbles,
    ) -> Result<(), MerkleRadixTrieError> {
        self.put_child(child_key, Default::default())
    }

    /// Removes the current node's child with the given prefix.
    pub fn remove_child(&mut self, child_prefix: &KeyNibbles) -> Result<(), MerkleRadixTrieError> {
        self.children[self.child_index(child_prefix)?] = None;
        Ok(())
    }

    pub fn put_value(
        &mut self,
        new_value: Vec<u8>,
    ) -> Result<Option<Vec<u8>>, MerkleRadixTrieError> {
        if self.root_data.is_some() {
            return Err(MerkleRadixTrieError::RootCantHaveValue);
        }
        Ok(mem::replace(&mut self.value, Some(new_value)))
    }

    pub fn iter_children(&self) -> Iter {
        self.into_iter()
    }

    pub fn iter_children_mut(&mut self) -> IterMut {
        self.into_iter()
    }

    fn can_hash(&self) -> bool {
        self.iter_children().all(|child| child.has_hash())
    }

    pub fn hash<H: HashOutput>(&self) -> Option<H> {
        self.can_hash().then(|| {
            let mut hasher = H::Builder::default();
            self.key.serialize(&mut hasher).unwrap();
            match (self.has_children(), &self.value) {
                (_, None) => {
                    hasher.write_u8(0).unwrap();
                }
                (false, Some(val)) => {
                    hasher.write_u8(1).unwrap();
                    val.serialize(&mut hasher).unwrap();
                }
                (true, Some(val)) => {
                    hasher.write_u8(2).unwrap();
                    let val_hash: Blake2bHash = val.hash();
                    val_hash.serialize(&mut hasher).unwrap();
                }
            }
            self.children.serialize(&mut hasher).unwrap();
            hasher.finish()
        })
    }

    pub fn hash_assert<H: HashOutput>(&self) -> H {
        self.hash()
            .expect("can only hash TrieNode with complete information about children")
    }
}

pub struct Iter<'a> {
    it: slice::Iter<'a, Option<TrieNodeChild>>,
}

impl<'a> Iter<'a> {
    pub fn from_children(children: &'a [Option<TrieNodeChild>; 16]) -> Iter<'a> {
        Iter {
            it: children.iter(),
        }
    }
}

impl<'a> Iterator for Iter<'a> {
    type Item = &'a TrieNodeChild;
    fn next(&mut self) -> Option<&'a TrieNodeChild> {
        loop {
            match self.it.next() {
                Some(Some(e)) => return Some(e),
                Some(None) => continue,
                None => return None,
            }
        }
    }
}

impl<'a> DoubleEndedIterator for Iter<'a> {
    fn next_back(&mut self) -> Option<Self::Item> {
        loop {
            match self.it.next_back() {
                Some(Some(e)) => return Some(e),
                Some(None) => continue,
                None => return None,
            }
        }
    }
}

impl<'a> IntoIterator for &'a TrieNode {
    type Item = &'a TrieNodeChild;
    type IntoIter = Iter<'a>;
    fn into_iter(self) -> Iter<'a> {
        Iter::from_children(&self.children)
    }
}

pub struct IterMut<'a> {
    it: slice::IterMut<'a, Option<TrieNodeChild>>,
}

impl<'a> Iterator for IterMut<'a> {
    type Item = &'a mut TrieNodeChild;
    fn next(&mut self) -> Option<&'a mut TrieNodeChild> {
        loop {
            match self.it.next() {
                Some(Some(e)) => return Some(e),
                Some(None) => continue,
                None => return None,
            }
        }
    }
}

impl<'a> DoubleEndedIterator for IterMut<'a> {
    fn next_back(&mut self) -> Option<Self::Item> {
        loop {
            match self.it.next_back() {
                Some(Some(e)) => return Some(e),
                Some(None) => continue,
                None => return None,
            }
        }
    }
}

impl<'a> IntoIterator for &'a mut TrieNode {
    type Item = &'a mut TrieNodeChild;
    type IntoIter = IterMut<'a>;
    fn into_iter(self) -> IterMut<'a> {
        IterMut {
            it: self.children.iter_mut(),
        }
    }
}

#[cfg(feature = "serde-derive")]
mod serde_derive {

    use std::fmt;

    use serde::{
        de::{Deserialize, Deserializer, Error, SeqAccess, Unexpected, Visitor},
        ser::{Serialize, SerializeStruct, Serializer},
    };
    use serde_bytes::ByteBuf;

    use super::{KeyNibbles, RootData, TrieNode, TrieNodeChild};

    struct TrieNodeVisitor;
    const FIELDS: &[&str] = &["flags", "root_data", "value", "child_count", "children"];

    impl<'de> Visitor<'de> for TrieNodeVisitor {
        type Value = TrieNode;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("struct TrieNode")
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let flags: u8 = seq
                .next_element()?
                .ok_or_else(|| A::Error::invalid_length(0, &self))?;
            let has_root_data = flags & 0x1 != 0;
            let has_value = flags & 0x2 != 0;
            let root_data: Option<RootData> = seq
                .next_element()?
                .ok_or_else(|| A::Error::invalid_length(1, &self))?;
            if has_root_data != root_data.is_some() {
                return Err(A::Error::invalid_value(
                    Unexpected::Other("Flags mismatch for root data"),
                    &self,
                )); // Mismatch flags for root data
            }
            let value: Option<Vec<u8>> = seq
                .next_element::<Option<ByteBuf>>()?
                .ok_or_else(|| A::Error::invalid_length(2, &self))?
                .map(|v| v.into_vec());
            if has_value != value.is_some() {
                return Err(A::Error::invalid_value(
                    Unexpected::Other("Flags mismatch for value"),
                    &self,
                )); // Mismatch flags for value
            }
            let exp_child_count: u8 = seq
                .next_element()?
                .ok_or_else(|| A::Error::invalid_length(3, &self))?;
            let children: [Option<TrieNodeChild>; 16] = seq
                .next_element()?
                .ok_or_else(|| A::Error::invalid_length(4, &self))?;
            let child_count: u8 = children
                .iter()
                .fold(0, |acc, child| acc + u8::from(!child.is_none()));
            if exp_child_count != child_count {
                return Err(A::Error::invalid_value(
                    Unexpected::Other("Unexpected number of children"),
                    &self,
                )); // bytes length too high
            }

            Ok(TrieNode {
                // Make it clear that the key needs to be changed after deserialization.
                key: KeyNibbles::BADBADBAD,
                root_data,
                value,
                children,
            })
        }
    }

    impl Serialize for TrieNode {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            let has_root_data = self.root_data.is_some();
            let has_value = self.value.is_some();
            let flags = (has_root_data as u8) | ((has_value as u8) << 1);
            let child_count: u8 = self
                .children
                .iter()
                .fold(0, |acc, child| acc + u8::from(!child.is_none()));

            let mut state = serializer.serialize_struct("TrieNode", FIELDS.len())?;
            state.serialize_field(FIELDS[0], &flags)?;
            state.serialize_field(FIELDS[1], &self.root_data)?;
            state.serialize_field(FIELDS[2], &self.value.as_deref().map(ByteBuf::from))?;
            state.serialize_field(FIELDS[3], &child_count)?;
            state.serialize_field(FIELDS[4], &self.children)?;
            state.end()
        }
    }

    impl<'de> Deserialize<'de> for TrieNode {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            deserializer.deserialize_struct("TrieNode", FIELDS, TrieNodeVisitor)
        }
    }
}

#[cfg(test)]
mod tests {
    use nimiq_hash::Hash;
    use nimiq_test_log::test;

    use super::*;

    #[test]
    fn child_index_works() {
        let key: KeyNibbles = "cfb986".parse().unwrap();

        let leaf_node = TrieNode::new_leaf(key.clone(), vec![66]);
        let branch_node = TrieNode::new_empty(key);

        let child_key_1 = "cfb986f5a".parse().unwrap();
        let child_key_2 = "cfb986ab9".parse().unwrap();
        let child_key_3 = "cfb9860f6".parse().unwrap();
        let child_key_4 = "cfb986d50".parse().unwrap();

        assert_eq!(branch_node.child_index(&child_key_1), Ok(15));
        assert_eq!(branch_node.child_index(&child_key_2), Ok(10));
        assert_eq!(branch_node.child_index(&child_key_3), Ok(0));
        assert_eq!(branch_node.child_index(&child_key_4), Ok(13));

        assert_eq!(branch_node.child_index(&child_key_1.slice(0, 7)), Ok(15));

        assert_eq!(leaf_node.child_index(&child_key_2), Ok(10));

        let child_key_5 = "c0b986d50".parse().unwrap();
        assert_eq!(
            branch_node.child_index(&child_key_5),
            Err(MerkleRadixTrieError::WrongPrefix)
        );
    }

    #[test]
    fn child_works() {
        let key: KeyNibbles = "cfb986".parse().unwrap();

        let leaf_node = TrieNode::new_leaf(key.clone(), vec![66]);
        let mut branch_node = TrieNode::new_empty(key);

        let child_key_1 = "cfb986f5a".parse().unwrap();
        branch_node
            .put_child(&child_key_1, "child_1".hash())
            .unwrap();

        let child_key_2 = "cfb986ab9".parse().unwrap();
        branch_node
            .put_child(&child_key_2, "child_2".hash())
            .unwrap();

        let child_key_3 = "cfb9860f6".parse().unwrap();
        branch_node
            .put_child(&child_key_3, "child_3".hash())
            .unwrap();

        let child_key_4 = "cfb986d50".parse().unwrap();
        branch_node
            .put_child(&child_key_4, "child_4".hash())
            .unwrap();

        assert_eq!(
            branch_node.child(&child_key_1).map(|c| &c.hash),
            Ok(&"child_1".hash())
        );
        assert_eq!(
            branch_node.child(&child_key_2).map(|c| &c.hash),
            Ok(&"child_2".hash())
        );
        assert_eq!(
            branch_node.child(&child_key_3).map(|c| &c.hash),
            Ok(&"child_3".hash())
        );
        assert_eq!(
            branch_node.child(&child_key_4).map(|c| &c.hash),
            Ok(&"child_4".hash())
        );

        assert_eq!(
            branch_node.child(&child_key_1.slice(0, 7)).map(|c| &c.hash),
            Ok(&"child_1".hash())
        );

        assert_eq!(
            leaf_node.child(&child_key_2).map(|c| &c.hash),
            Err(MerkleRadixTrieError::ChildDoesNotExist)
        );

        let child_key_5 = "c0b986d50".parse().unwrap();
        assert_eq!(
            branch_node.child(&child_key_5).map(|c| &c.hash),
            Err(MerkleRadixTrieError::WrongPrefix)
        );
    }

    #[test]
    fn child_key_works() {
        let key: KeyNibbles = "cfb986".parse().unwrap();

        let leaf_node = TrieNode::new_leaf(key.clone(), vec![66]);
        let mut branch_node = TrieNode::new_empty(key);

        let child_key_1 = "cfb986f5a".parse().unwrap();
        branch_node
            .put_child(&child_key_1, "child_1".hash())
            .unwrap();

        let child_key_2 = "cfb986ab9".parse().unwrap();
        branch_node
            .put_child(&child_key_2, "child_2".hash())
            .unwrap();

        let child_key_3 = "cfb9860f6".parse().unwrap();
        branch_node
            .put_child(&child_key_3, "child_3".hash())
            .unwrap();

        let child_key_4 = "cfb986d50".parse().unwrap();
        branch_node
            .put_child(&child_key_4, "child_4".hash())
            .unwrap();

        assert_eq!(
            branch_node.child_key(&child_key_1, &None),
            Ok(child_key_1.clone())
        );
        assert_eq!(
            branch_node.child_key(&child_key_2, &None),
            Ok(child_key_2.clone())
        );
        assert_eq!(
            branch_node.child_key(&child_key_3, &None),
            Ok(child_key_3.clone())
        );
        assert_eq!(
            branch_node.child_key(&child_key_4, &None),
            Ok(child_key_4.clone())
        );

        assert_eq!(
            branch_node.child_key(&child_key_1.slice(0, 7), &None),
            Ok(child_key_1)
        );

        assert_eq!(
            leaf_node.child_key(&child_key_2, &None),
            Err(MerkleRadixTrieError::ChildDoesNotExist)
        );

        let child_key_5 = "c0b986d50".parse().unwrap();
        assert_eq!(
            branch_node.child_key(&child_key_5, &None),
            Err(MerkleRadixTrieError::WrongPrefix)
        );
    }

    #[test]
    fn put_remove_child_works() {
        let key: KeyNibbles = "cfb986".parse().unwrap();

        let mut node = TrieNode::new_leaf(key, vec![66]);

        let child_key_1 = "cfb986f5a".parse().unwrap();
        node.put_child(&child_key_1, "child_1".hash()).unwrap();

        let child_key_2 = "cfb986ab9".parse().unwrap();
        node.put_child(&child_key_2, "child_2".hash()).unwrap();

        let child_key_3 = "cfb9860f6".parse().unwrap();
        node.put_child(&child_key_3, "child_3".hash()).unwrap();

        let child_key_4 = "cfb986d50".parse().unwrap();
        node.put_child(&child_key_4, "child_4".hash()).unwrap();

        node.remove_child(&child_key_1).unwrap();
        assert_eq!(
            node.child(&child_key_1),
            Err(MerkleRadixTrieError::ChildDoesNotExist)
        );

        node.remove_child(&child_key_2).unwrap();
        assert_eq!(
            node.child(&child_key_2),
            Err(MerkleRadixTrieError::ChildDoesNotExist)
        );

        node.remove_child(&child_key_3).unwrap();
        assert_eq!(
            node.child(&child_key_3),
            Err(MerkleRadixTrieError::ChildDoesNotExist)
        );

        node.remove_child(&child_key_4).unwrap();
        assert_eq!(
            node.child(&child_key_4),
            Err(MerkleRadixTrieError::ChildDoesNotExist)
        );

        assert_eq!(node.value, Some(vec![66]));
    }

    #[test]
    fn put_value_works() {
        let key: KeyNibbles = "cfb986".parse().unwrap();

        let mut leaf_node = TrieNode::new_leaf(key.clone(), vec![66]);
        assert_eq!(leaf_node.value, Some(vec![66]));
        leaf_node.put_value(vec![99]).unwrap();
        assert_eq!(leaf_node.value, Some(vec![99]));

        let mut hybrid_node = TrieNode::new_leaf(key.clone(), vec![66]);
        assert_eq!(hybrid_node.value, Some(vec![66]));
        hybrid_node.put_value(vec![99]).unwrap();
        assert_eq!(hybrid_node.value, Some(vec![99]));

        let mut branch_node = TrieNode::new_empty(key);
        assert_eq!(branch_node.value, None);
        branch_node.put_value(vec![99]).unwrap();
        assert_eq!(branch_node.value, Some(vec![99]));

        let mut root_node = TrieNode::new_root();
        assert_eq!(root_node.value, None);
        assert_eq!(
            root_node.put_value(vec![99]),
            Err(MerkleRadixTrieError::RootCantHaveValue)
        );
        assert_eq!(root_node.value, None);
    }

    #[test]
    fn remove_value_works() {
        let key: KeyNibbles = "cfb986".parse().unwrap();

        let mut leaf_node = TrieNode::new_leaf(key.clone(), vec![66]);
        assert_eq!(leaf_node.value, Some(vec![66]));
        leaf_node.value = None;
        assert!(leaf_node.is_empty());

        let branch_node = TrieNode::new_empty(key);
        assert_eq!(branch_node.value, None);

        let root_node = TrieNode::new_root();
        assert_eq!(root_node.value, None);
    }
}
