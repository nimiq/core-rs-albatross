use std::io;
use std::ops;
use std::ops::RangeFrom;
use std::slice;

use log::error;

use beserial::{
    Deserialize, DeserializeWithLength, ReadBytesExt, Serialize, SerializeWithLength,
    SerializingError, WriteBytesExt,
};
use nimiq_database_value::{FromDatabaseValue, IntoDatabaseValue};
use nimiq_hash::{Blake2bHash, HashOutput, Hasher, SerializeContent};

use crate::{key_nibbles::KeyNibbles, trie::error::MerkleRadixTrieError};

/// A struct representing a node in the Merkle Radix Trie. It can be either a branch node, which has
/// only references to its children, a leaf node, which contains a value or a hybrid node, which has
/// both children and a value. A branch/hybrid node can have up to 16 children, since we represent
/// the keys in hexadecimal form, each child represents a different hexadecimal character.
#[derive(Clone, Debug)]
pub struct TrieNode {
    // This is not serialized.
    pub key: KeyNibbles,
    // The root data is not included when the `TrieNode` is hashed.
    pub root_data: Option<RootData>,
    pub value: Option<Vec<u8>>,
    pub children: [Option<TrieNodeChild>; 16],
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RootData {
    pub incomplete: Option<ops::RangeFrom<KeyNibbles>>,
    pub num_branches: u64,
    pub num_hybrids: u64,
    pub num_leaves: u64,
}

/// A struct representing the child of a node. It just contains the child's suffix (the part of the
/// child's key that is different from its parent) and its hash.
#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Debug, Serialize, Deserialize)]
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

    pub fn put_value(&mut self, new_value: Vec<u8>) -> Result<(), MerkleRadixTrieError> {
        if self.root_data.is_some() {
            return Err(MerkleRadixTrieError::RootCantHaveValue);
        }
        self.value = Some(new_value);
        Ok(())
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
            self.serialize_content(&mut hasher).unwrap();
            hasher.finish()
        })
    }

    pub fn hash_assert<H: HashOutput>(&self) -> H {
        self.hash()
            .expect("can only hash TrieNode with complete information about children")
    }
}

impl Deserialize for TrieNode {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let flags = reader.read_u8()?;
        let has_root_data = flags & 0x1 != 0;
        let has_value = flags & 0x2 != 0;

        let root_data = if has_root_data {
            Some(Deserialize::deserialize(reader)?)
        } else {
            None
        };
        let value = if has_value {
            Some(DeserializeWithLength::deserialize::<u16, _>(reader)?)
        } else {
            None
        };

        let child_count: u8 = Deserialize::deserialize(reader)?;

        let mut children = NO_CHILDREN;

        for _ in 0..child_count {
            let child: TrieNodeChild = Deserialize::deserialize(reader)?;

            if let Some(i) = child.suffix.get(0) {
                children[i] = Some(child);
            } else {
                return Err(io::Error::from(io::ErrorKind::InvalidData).into());
            }
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
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let has_root_data = self.root_data.is_some();
        let has_value = self.value.is_some();

        let flags = (has_root_data as u8) | ((has_value as u8) << 1);

        let mut size = 1;
        writer.write_u8(flags)?;
        if let Some(root_data) = &self.root_data {
            size += root_data.serialize(writer)?;
        }
        if let Some(value) = &self.value {
            size += value.serialize::<u16, _>(writer)?;
        }

        let child_count: u8 = self
            .children
            .iter()
            .fold(0, |acc, child| acc + u8::from(!child.is_none()));
        Serialize::serialize(&child_count, writer)?;

        for child in self.children.iter().flatten() {
            size += Serialize::serialize(&child, writer)?;
        }
        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size = 1;
        if let Some(root_data) = &self.root_data {
            size += root_data.serialized_size();
        }
        if let Some(value) = &self.value {
            size += value.serialized_size::<u16>();
        }
        size += 1; // count

        for child in self.children.iter().flatten() {
            size += Serialize::serialized_size(&child);
        }
        size
    }
}

impl SerializeContent for TrieNode {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> {
        let mut size = 0;
        size += self.key.serialize(writer)?;
        if let Some(v) = &self.value {
            writer.write_u8(1)?;
            size += v.serialize::<u16, _>(writer)?;
        } else {
            writer.write_u8(0)?;
        }
        size += self.children.serialize(writer)?;
        Ok(size)
    }
}

impl IntoDatabaseValue for TrieNode {
    fn database_byte_size(&self) -> usize {
        self.serialized_size()
    }

    fn copy_into_database(&self, mut bytes: &mut [u8]) {
        Serialize::serialize(&self, &mut bytes).unwrap();
    }
}

impl FromDatabaseValue for TrieNode {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self>
    where
        Self: Sized,
    {
        let mut cursor = io::Cursor::new(bytes);
        Ok(Deserialize::deserialize(&mut cursor)?)
    }
}

pub struct Iter<'a> {
    it: slice::Iter<'a, Option<TrieNodeChild>>,
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
        Iter {
            it: self.children.iter(),
        }
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
