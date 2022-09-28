use std::io;
use std::iter;
use std::slice;

use log::error;

use beserial::{Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};
use nimiq_database::{FromDatabaseValue, IntoDatabaseValue};
use nimiq_hash::{Blake2bHash, Hash, SerializeContent};

use crate::error::MerkleRadixTrieError;
use crate::key_nibbles::KeyNibbles;

/// A struct representing a node in the Merkle Radix Trie. It can be either a branch node, which has
/// only references to its children, a leaf node, which contains a value or a hybrid node, which has
/// both children and a value. A branch/hybrid node can have up to 16 children, since we represent
/// the keys in hexadecimal form, each child represents a different hexadecimal character.
#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum TrieNode {
    Root {
        num_branches: u64,
        num_leaves: u64,
        children: TrieNodeChildren,
    },
    Branch {
        key: KeyNibbles,
        children: TrieNodeChildren,
    },
    Leaf {
        key: KeyNibbles,
        #[beserial(len_type(u16))]
        value: Vec<u8>,
    },
    Hybrid {
        key: KeyNibbles,
        #[beserial(len_type(u16))]
        value: Vec<u8>,
        children: TrieNodeChildren,
    },
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct TrieNodeChildren([Option<TrieNodeChild>; 16]);

/// A struct representing the child of a node. It just contains the child's suffix (the part of the
/// child's key that is different from its parent) and its hash.
#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Debug, Serialize, Deserialize)]
pub struct TrieNodeChild {
    pub suffix: KeyNibbles,
    pub hash: Blake2bHash,
}

pub const NO_CHILDREN: TrieNodeChildren = TrieNodeChildren([
    None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None,
]);

impl TrieNode {
    /// Creates a new leaf node.
    pub fn new_leaf<T: Serialize>(key: KeyNibbles, value: T) -> Self {
        TrieNode::Leaf {
            key,
            value: value.serialize_to_vec(),
        }
    }

    /// Creates an empty root node
    pub fn new_root() -> Self {
        TrieNode::Root {
            children: NO_CHILDREN,
            num_branches: 1,
            num_leaves: 0,
        }
    }

    /// Creates a new empty branch node.
    pub fn new_branch(key: KeyNibbles) -> Self {
        TrieNode::Branch {
            key,
            children: NO_CHILDREN,
        }
    }

    /// Creates a new hybrid node with no children.
    pub fn new_hybrid<T: Serialize>(key: KeyNibbles, value: T) -> Self {
        TrieNode::Hybrid {
            key,
            value: value.serialize_to_vec(),
            children: NO_CHILDREN,
        }
    }

    /// Checks if this node contains a value.
    pub fn has_value(&self) -> bool {
        match self {
            TrieNode::Leaf { .. } | TrieNode::Hybrid { .. } => true,
            TrieNode::Root { .. } | TrieNode::Branch { .. } => false,
        }
    }

    /// Checks if this node has children.
    pub fn has_children(&self) -> bool {
        match self {
            TrieNode::Root { .. } | TrieNode::Branch { .. } | TrieNode::Hybrid { .. } => true,
            TrieNode::Leaf { .. } => false,
        }
    }

    /// Returns the key of a node.
    pub fn key(&self) -> &KeyNibbles {
        match self {
            TrieNode::Root { .. } => &KeyNibbles::ROOT,
            TrieNode::Leaf { ref key, .. }
            | TrieNode::Branch { ref key, .. }
            | TrieNode::Hybrid { ref key, .. } => key,
        }
    }

    /// Returns the children of a node, if it is not a leaf node.
    pub fn children(&self) -> Result<&[Option<TrieNodeChild>; 16], MerkleRadixTrieError> {
        match self {
            TrieNode::Root { ref children, .. }
            | TrieNode::Branch { ref children, .. }
            | TrieNode::Hybrid { ref children, .. } => Ok(&children.0),
            TrieNode::Leaf { .. } => Err(MerkleRadixTrieError::LeavesHaveNoChildren),
        }
    }

    /// Returns the children of a node, if it is not a leaf node.
    pub fn children_mut(
        &mut self,
    ) -> Result<&mut [Option<TrieNodeChild>; 16], MerkleRadixTrieError> {
        match self {
            TrieNode::Root {
                ref mut children, ..
            }
            | TrieNode::Branch {
                ref mut children, ..
            }
            | TrieNode::Hybrid {
                ref mut children, ..
            } => Ok(&mut children.0),
            TrieNode::Leaf { .. } => Err(MerkleRadixTrieError::LeavesHaveNoChildren),
        }
    }

    /// Returns the value of a node, if it is a leaf or hybrid node.
    pub fn value<T: Deserialize>(&self) -> Result<T, MerkleRadixTrieError> {
        match self {
            TrieNode::Leaf { value, .. } | TrieNode::Hybrid { value, .. } => {
                Ok(T::deserialize(&mut &value[..]).unwrap())
            }
            TrieNode::Root { .. } | TrieNode::Branch { .. } => {
                Err(MerkleRadixTrieError::BranchesHaveNoValue)
            }
        }
    }

    /// Returns the child index of the given prefix in the current node. If the current node has the
    /// key "31f6d" and the given child key is "31f6d925ca" (both are in hexadecimal) then the child
    /// index is 9.
    pub fn get_child_index(
        &self,
        child_prefix: &KeyNibbles,
    ) -> Result<usize, MerkleRadixTrieError> {
        if !self.key().is_prefix_of(child_prefix) {
            error!(
                "Child's prefix {} is not a prefix of the node with key {}!",
                child_prefix,
                self.key()
            );
            return Err(MerkleRadixTrieError::WrongPrefix);
        }

        // Key length has to be smaller or equal to the child prefix length, so this should never panic!
        Ok(child_prefix.get(self.key().len()).unwrap())
    }

    /// Returns the hash of the current node's child with the given prefix.
    pub fn get_child_hash(
        &self,
        child_prefix: &KeyNibbles,
    ) -> Result<&Blake2bHash, MerkleRadixTrieError> {
        if let Some(child) = &self.children()?[self.get_child_index(child_prefix)?] {
            return Ok(&child.hash);
        }
        Err(MerkleRadixTrieError::ChildDoesNotExist)
    }

    /// Returns the key of the current node's child with the given prefix.
    pub fn get_child_key(
        &self,
        child_prefix: &KeyNibbles,
    ) -> Result<KeyNibbles, MerkleRadixTrieError> {
        if let Some(ref child) = self.children()?[self.get_child_index(child_prefix)?] {
            return Ok(self.key() + &child.suffix);
        }
        Err(MerkleRadixTrieError::ChildDoesNotExist)
    }

    /// Add a child, with the given key and hash, to the current node. Returns the modified node.
    pub fn put_child(
        mut self,
        child_key: &KeyNibbles,
        child_hash: Blake2bHash,
    ) -> Result<Self, MerkleRadixTrieError> {
        let child_index = self.get_child_index(child_key)?;
        let suffix = child_key.suffix(self.key().len() as u8);

        // Turn leaf node into a hybrid node.
        self = match self {
            TrieNode::Leaf { key, value } => TrieNode::Hybrid {
                key,
                value,
                children: NO_CHILDREN,
            },
            other => other,
        };

        self.children_mut()?[child_index] = Some(TrieNodeChild {
            suffix,
            hash: child_hash,
        });

        Ok(self)
    }

    /// Removes the child with the given key from the current node. If successful, it returns the
    /// modified node.
    pub fn remove_child(mut self, child_key: &KeyNibbles) -> Result<Self, MerkleRadixTrieError> {
        let child_index = self.get_child_index(child_key)?;

        self.children_mut()?[child_index] = None;

        // If there are no more children, turn this hybrid node into a leaf node.
        self = match self {
            TrieNode::Hybrid {
                key,
                value,
                children,
            } if children.0.iter().all(|child| child.is_none()) => TrieNode::Leaf { key, value },
            other => other,
        };

        Ok(self)
    }

    /// Adds, or overwrites, the value at the given node. If successful, it returns  the modified
    /// node.
    pub fn put_value<T: Serialize>(mut self, new_value: T) -> Result<Self, MerkleRadixTrieError> {
        match self {
            // Turn branch node into a hybrid node.
            TrieNode::Branch { key, children } => Ok(TrieNode::Hybrid {
                key,
                value: new_value.serialize_to_vec(),
                children,
            }),
            TrieNode::Leaf { ref mut value, .. } | TrieNode::Hybrid { ref mut value, .. } => {
                *value = new_value.serialize_to_vec();
                Ok(self)
            }
            TrieNode::Root { .. } => Err(MerkleRadixTrieError::RootCantHaveValue),
        }
    }

    /// Removes the value at the given node. Returns the modified node or None if the node is a
    /// leaf node.
    pub fn remove_value(self) -> Option<Self> {
        match self {
            TrieNode::Leaf { .. } => None,
            TrieNode::Hybrid { key, children, .. } => {
                // Turn this hybrid node into a branch node.
                Some(TrieNode::Branch { key, children })
            }
            // TODO Error instead silently doing nothing?
            TrieNode::Root { .. } | TrieNode::Branch { .. } => Some(self),
        }
    }

    pub fn iter_children(&self) -> Iter {
        self.into_iter()
    }

    pub fn iter_children_mut(&mut self) -> IterMut {
        self.into_iter()
    }
}

impl SerializeContent for TrieNode {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> {
        Ok(self.serialize(writer)?)
    }
}

impl Hash for TrieNode {}

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

#[allow(clippy::type_complexity)]
type AccountsTrieNodeIter<'a> = iter::FilterMap<
    slice::Iter<'a, Option<TrieNodeChild>>,
    fn(&Option<TrieNodeChild>) -> Option<&TrieNodeChild>,
>;

pub struct Iter<'a> {
    it: AccountsTrieNodeIter<'a>,
}

impl<'a> Iterator for Iter<'a> {
    type Item = &'a TrieNodeChild;

    fn next(&mut self) -> Option<&'a TrieNodeChild> {
        self.it.next()
    }
}

impl<'a> IntoIterator for &'a TrieNode {
    type Item = &'a TrieNodeChild;
    type IntoIter = Iter<'a>;

    fn into_iter(self) -> Iter<'a> {
        fn to_slice<T>(array: &[T; 16]) -> &[T] {
            array
        }
        Iter {
            it: self
                .children()
                .map(to_slice)
                .unwrap_or(&[])
                .iter()
                .filter_map(Option::as_ref),
        }
    }
}

type AccountsTrieNodeChildFilterMap<'a> = iter::FilterMap<
    slice::IterMut<'a, Option<TrieNodeChild>>,
    fn(&mut Option<TrieNodeChild>) -> Option<&mut TrieNodeChild>,
>;

pub struct IterMut<'a> {
    it: AccountsTrieNodeChildFilterMap<'a>,
}

impl<'a> Iterator for IterMut<'a> {
    type Item = &'a mut TrieNodeChild;

    fn next(&mut self) -> Option<&'a mut TrieNodeChild> {
        self.it.next()
    }
}

impl<'a> IntoIterator for &'a mut TrieNode {
    type Item = &'a mut TrieNodeChild;
    type IntoIter = IterMut<'a>;

    fn into_iter(self) -> IterMut<'a> {
        fn to_slice<T>(array: &mut [T; 16]) -> &mut [T] {
            array
        }
        IterMut {
            it: self
                .children_mut()
                .map(to_slice)
                .unwrap_or(&mut [])
                .iter_mut()
                .filter_map(Option::as_mut),
        }
    }
}

impl Serialize for TrieNodeChildren {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = 0;
        let child_count: u8 = self
            .0
            .iter()
            .fold(0, |acc, child| acc + if child.is_none() { 0 } else { 1 });
        size += Serialize::serialize(&child_count, writer)?;
        for child in self.0.iter().flatten() {
            size += Serialize::serialize(&child, writer)?;
        }
        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size = /*count*/ 1;
        for child in self.0.iter().flatten() {
            size += Serialize::serialized_size(&child);
        }
        size
    }
}

impl Deserialize for TrieNodeChildren {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let child_count: u8 = Deserialize::deserialize(reader)?;
        let mut children = NO_CHILDREN;
        for _ in 0..child_count {
            let child: TrieNodeChild = Deserialize::deserialize(reader)?;
            if let Some(i) = child.suffix.get(0) {
                children.0[i] = Some(child);
            } else {
                return Err(io::Error::from(io::ErrorKind::InvalidData).into());
            }
        }
        Ok(children)
    }
}

#[cfg(test)]
mod tests {
    use nimiq_test_log::test;

    use super::*;

    #[test]
    fn get_child_index_works() {
        let key: KeyNibbles = "cfb986".parse().unwrap();

        let leaf_node = TrieNode::new_leaf::<u32>(key.clone(), 666);
        let branch_node = TrieNode::new_branch(key);

        let child_key_1 = "cfb986f5a".parse().unwrap();
        let child_key_2 = "cfb986ab9".parse().unwrap();
        let child_key_3 = "cfb9860f6".parse().unwrap();
        let child_key_4 = "cfb986d50".parse().unwrap();

        assert_eq!(branch_node.get_child_index(&child_key_1), Ok(15));
        assert_eq!(branch_node.get_child_index(&child_key_2), Ok(10));
        assert_eq!(branch_node.get_child_index(&child_key_3), Ok(0));
        assert_eq!(branch_node.get_child_index(&child_key_4), Ok(13));

        assert_eq!(
            branch_node.get_child_index(&child_key_1.slice(0, 7)),
            Ok(15)
        );

        assert_eq!(leaf_node.get_child_index(&child_key_2), Ok(10));

        let child_key_5 = "c0b986d50".parse().unwrap();
        assert_eq!(
            branch_node.get_child_index(&child_key_5),
            Err(MerkleRadixTrieError::WrongPrefix)
        );
    }

    #[test]
    fn get_child_hash_works() {
        let key: KeyNibbles = "cfb986".parse().unwrap();

        let leaf_node = TrieNode::new_leaf::<u32>(key.clone(), 666);
        let mut branch_node = TrieNode::new_branch(key);

        let child_key_1 = "cfb986f5a".parse().unwrap();
        branch_node = branch_node
            .put_child(&child_key_1, "child_1".hash())
            .unwrap();

        let child_key_2 = "cfb986ab9".parse().unwrap();
        branch_node = branch_node
            .put_child(&child_key_2, "child_2".hash())
            .unwrap();

        let child_key_3 = "cfb9860f6".parse().unwrap();
        branch_node = branch_node
            .put_child(&child_key_3, "child_3".hash())
            .unwrap();

        let child_key_4 = "cfb986d50".parse().unwrap();
        branch_node = branch_node
            .put_child(&child_key_4, "child_4".hash())
            .unwrap();

        assert_eq!(
            branch_node.get_child_hash(&child_key_1),
            Ok(&"child_1".hash())
        );
        assert_eq!(
            branch_node.get_child_hash(&child_key_2),
            Ok(&"child_2".hash())
        );
        assert_eq!(
            branch_node.get_child_hash(&child_key_3),
            Ok(&"child_3".hash())
        );
        assert_eq!(
            branch_node.get_child_hash(&child_key_4),
            Ok(&"child_4".hash())
        );

        assert_eq!(
            branch_node.get_child_hash(&child_key_1.slice(0, 7)),
            Ok(&"child_1".hash())
        );

        assert_eq!(
            leaf_node.get_child_hash(&child_key_2),
            Err(MerkleRadixTrieError::LeavesHaveNoChildren)
        );

        let child_key_5 = "c0b986d50".parse().unwrap();
        assert_eq!(
            branch_node.get_child_hash(&child_key_5),
            Err(MerkleRadixTrieError::WrongPrefix)
        );
    }

    #[test]
    fn get_child_key_works() {
        let key: KeyNibbles = "cfb986".parse().unwrap();

        let leaf_node = TrieNode::new_leaf::<u32>(key.clone(), 666);
        let mut branch_node = TrieNode::new_branch(key);

        let child_key_1 = "cfb986f5a".parse().unwrap();
        branch_node = branch_node
            .put_child(&child_key_1, "child_1".hash())
            .unwrap();

        let child_key_2 = "cfb986ab9".parse().unwrap();
        branch_node = branch_node
            .put_child(&child_key_2, "child_2".hash())
            .unwrap();

        let child_key_3 = "cfb9860f6".parse().unwrap();
        branch_node = branch_node
            .put_child(&child_key_3, "child_3".hash())
            .unwrap();

        let child_key_4 = "cfb986d50".parse().unwrap();
        branch_node = branch_node
            .put_child(&child_key_4, "child_4".hash())
            .unwrap();

        assert_eq!(
            branch_node.get_child_key(&child_key_1),
            Ok(child_key_1.clone())
        );
        assert_eq!(
            branch_node.get_child_key(&child_key_2),
            Ok(child_key_2.clone())
        );
        assert_eq!(
            branch_node.get_child_key(&child_key_3),
            Ok(child_key_3.clone())
        );
        assert_eq!(
            branch_node.get_child_key(&child_key_4),
            Ok(child_key_4.clone())
        );

        assert_eq!(
            branch_node.get_child_key(&child_key_1.slice(0, 7)),
            Ok(child_key_1)
        );

        assert_eq!(
            leaf_node.get_child_key(&child_key_2),
            Err(MerkleRadixTrieError::LeavesHaveNoChildren)
        );

        let child_key_5 = "c0b986d50".parse().unwrap();
        assert_eq!(
            branch_node.get_child_key(&child_key_5),
            Err(MerkleRadixTrieError::WrongPrefix)
        );
    }

    #[test]
    fn put_remove_child_works() {
        let key: KeyNibbles = "cfb986".parse().unwrap();

        let mut node = TrieNode::new_leaf(key, 666u32);

        let child_key_1 = "cfb986f5a".parse().unwrap();
        node = node.put_child(&child_key_1, "child_1".hash()).unwrap();

        let child_key_2 = "cfb986ab9".parse().unwrap();
        node = node.put_child(&child_key_2, "child_2".hash()).unwrap();

        let child_key_3 = "cfb9860f6".parse().unwrap();
        node = node.put_child(&child_key_3, "child_3".hash()).unwrap();

        let child_key_4 = "cfb986d50".parse().unwrap();
        node = node.put_child(&child_key_4, "child_4".hash()).unwrap();

        node = node.remove_child(&child_key_1).unwrap();
        assert_eq!(
            node.get_child_hash(&child_key_1),
            Err(MerkleRadixTrieError::ChildDoesNotExist)
        );

        node = node.remove_child(&child_key_2).unwrap();
        assert_eq!(
            node.get_child_hash(&child_key_2),
            Err(MerkleRadixTrieError::ChildDoesNotExist)
        );

        node = node.remove_child(&child_key_3).unwrap();
        assert_eq!(
            node.get_child_hash(&child_key_3),
            Err(MerkleRadixTrieError::ChildDoesNotExist)
        );

        node = node.remove_child(&child_key_4).unwrap();
        assert_eq!(
            node.get_child_hash(&child_key_4),
            Err(MerkleRadixTrieError::LeavesHaveNoChildren)
        );

        assert_eq!(node.value(), Ok(666u32));
    }

    #[test]
    fn put_value_works() {
        let key: KeyNibbles = "cfb986".parse().unwrap();

        let leaf_node = TrieNode::new_leaf::<u32>(key.clone(), 666);
        assert_eq!(leaf_node.value(), Ok(666u32));
        let leaf_node = leaf_node.put_value(999).unwrap();
        assert_eq!(leaf_node.value(), Ok(999u32));

        let hybrid_node = TrieNode::new_hybrid::<u32>(key.clone(), 666);
        assert_eq!(hybrid_node.value(), Ok(666u32));
        let hybrid_node = hybrid_node.put_value(999).unwrap();
        assert_eq!(hybrid_node.value(), Ok(999u32));

        let branch_node = TrieNode::new_branch(key);
        assert_eq!(
            branch_node.value::<u32>(),
            Err(MerkleRadixTrieError::BranchesHaveNoValue)
        );
        let branch_node = branch_node.put_value(999).unwrap();
        assert_eq!(branch_node.value(), Ok(999u32));

        let root_node = TrieNode::new_root();
        assert_eq!(
            root_node.value::<u32>(),
            Err(MerkleRadixTrieError::BranchesHaveNoValue)
        );
        assert_eq!(
            root_node.put_value(999),
            Err(MerkleRadixTrieError::RootCantHaveValue)
        );
    }

    #[test]
    fn remove_value_works() {
        let key: KeyNibbles = "cfb986".parse().unwrap();

        let leaf_node = TrieNode::new_leaf::<u32>(key.clone(), 666);
        assert_eq!(leaf_node.value(), Ok(666u32));
        let leaf_node = leaf_node.remove_value();
        assert_eq!(leaf_node, None);

        let hybrid_node = TrieNode::new_hybrid::<u32>(key.clone(), 666);
        assert_eq!(hybrid_node.value(), Ok(666u32));
        let hybrid_node = hybrid_node.remove_value().unwrap();
        assert_eq!(
            hybrid_node.value::<u32>(),
            Err(MerkleRadixTrieError::BranchesHaveNoValue)
        );

        let branch_node = TrieNode::new_branch(key);
        assert_eq!(
            branch_node.value::<u32>(),
            Err(MerkleRadixTrieError::BranchesHaveNoValue)
        );
        let branch_node = branch_node.remove_value().unwrap();
        assert_eq!(
            branch_node.value::<u32>(),
            Err(MerkleRadixTrieError::BranchesHaveNoValue)
        );

        let root_node = TrieNode::new_root();
        assert_eq!(
            root_node.value::<u32>(),
            Err(MerkleRadixTrieError::BranchesHaveNoValue)
        );
        let branch_node = branch_node.remove_value().unwrap();
        assert_eq!(
            branch_node.value::<u32>(),
            Err(MerkleRadixTrieError::BranchesHaveNoValue)
        );
    }
}
