use std::io;
use std::iter;
use std::slice;

use log::error;

use beserial::{Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};
use nimiq_database_value::{FromDatabaseValue, IntoDatabaseValue};
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
    RootNode {
        num_branches: u64,
        num_leaves: u64,
        children: TrieNodeChildren,
    },
    BranchNode {
        key: KeyNibbles,
        children: TrieNodeChildren,
    },
    LeafNode {
        key: KeyNibbles,
        #[beserial(len_type(u16))]
        value: Vec<u8>,
    },
    HybridNode {
        key: KeyNibbles,
        #[beserial(len_type(u16))]
        value: Vec<u8>,
        children: TrieNodeChildren,
    },
}

pub type TrieNodeChildren = [Option<TrieNodeChild>; 16];

// #[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
// pub struct TrieNodeChildren([Option<TrieNodeChild>; 16]);

/// A struct representing the child of a node. It just contains the child's suffix (the part of the
/// child's key that is different from its parent) and its hash.
#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Debug, Serialize, Deserialize)]
pub struct TrieNodeChild {
    pub suffix: KeyNibbles,
    pub hash: Blake2bHash,
}

pub const NO_CHILDREN: TrieNodeChildren = [
    None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None,
];

impl TrieNode {
    /// Creates a new leaf node.
    pub fn new_leaf<T: Serialize>(key: KeyNibbles, value: T) -> Self {
        TrieNode::LeafNode {
            key,
            value: value.serialize_to_vec(),
        }
    }

    /// Creates an empty root node
    pub fn new_root() -> Self {
        TrieNode::RootNode {
            children: NO_CHILDREN,
            num_branches: 1,
            num_leaves: 0,
        }
    }

    /// Creates a new empty branch node.
    pub fn new_branch(key: KeyNibbles) -> Self {
        TrieNode::BranchNode {
            key,
            children: NO_CHILDREN,
        }
    }

    /// Creates a new hybrid node with no children.
    pub fn new_hybrid<T: Serialize>(key: KeyNibbles, value: T) -> Self {
        TrieNode::HybridNode {
            key,
            value: value.serialize_to_vec(),
            children: NO_CHILDREN,
        }
    }

    /// Checks if the node is a leaf node.
    pub fn is_leaf(&self) -> bool {
        match self {
            TrieNode::LeafNode { .. } => true,
            _ => false,
        }
    }

    /// Checks if the node is a branch node.
    pub fn is_branch(&self) -> bool {
        match self {
            TrieNode::BranchNode { .. } => true,
            _ => false,
        }
    }

    /// Checks if the node is a hybrid node.
    pub fn is_hybrid(&self) -> bool {
        match self {
            TrieNode::HybridNode { .. } => true,
            _ => false,
        }
    }

    /// Check if this node contains a value.
    pub fn has_value(&self) -> bool {
        self.is_leaf() || self.is_hybrid()
    }

    /// Returns the key of a node.
    pub fn key(&self) -> &KeyNibbles {
        match self {
            TrieNode::RootNode { .. } => &KeyNibbles::ROOT,
            TrieNode::LeafNode { ref key, .. }
            | TrieNode::BranchNode { ref key, .. }
            | TrieNode::HybridNode { ref key, .. } => key,
        }
    }

    pub fn children(&self) -> Result<&TrieNodeChildren, MerkleRadixTrieError> {
        match self {
            TrieNode::RootNode { ref children, .. }
            | TrieNode::BranchNode { ref children, .. }
            | TrieNode::HybridNode { ref children, .. } => Ok(children),
            TrieNode::LeafNode { .. } => Err(MerkleRadixTrieError::LeavesHaveNoChildren),
        }
    }

    pub fn children_mut(&mut self) -> Result<&mut TrieNodeChildren, MerkleRadixTrieError> {
        match self {
            TrieNode::RootNode {
                ref mut children, ..
            }
            | TrieNode::BranchNode {
                ref mut children, ..
            }
            | TrieNode::HybridNode {
                ref mut children, ..
            } => Ok(children),
            TrieNode::LeafNode { .. } => Err(MerkleRadixTrieError::LeavesHaveNoChildren),
        }
    }

    /// Returns the value of a node, if it is a leaf or hybrid node.
    pub fn value<T: Deserialize>(&self) -> Result<T, MerkleRadixTrieError> {
        match self {
            TrieNode::LeafNode { ref value, .. } | TrieNode::HybridNode { ref value, .. } => {
                Ok(T::deserialize(&mut &value[..]).unwrap())
            }
            TrieNode::RootNode { .. } | TrieNode::BranchNode { .. } => {
                error!(
                    "Node with key {} is a branch node and so it can't have a value!",
                    self.key()
                );
                Err(MerkleRadixTrieError::BranchesHaveNoValue)
            }
        }
    }

    /// Returns the value of a node, if it is a leaf or hybrid node.
    pub fn raw_value_mut(&mut self) -> Result<&mut Vec<u8>, MerkleRadixTrieError> {
        match self {
            TrieNode::LeafNode { ref mut value, .. }
            | TrieNode::HybridNode { ref mut value, .. } => Ok(value),
            TrieNode::RootNode { .. } | TrieNode::BranchNode { .. } => {
                error!(
                    "Node with key {} is a branch node and so it can't have a value!",
                    self.key()
                );
                Err(MerkleRadixTrieError::BranchesHaveNoValue)
            }
        }
    }

    /// Returns the value of a node, if it is a leaf or hybrid node.
    pub fn into_raw_value(self) -> Result<Vec<u8>, MerkleRadixTrieError> {
        match self {
            TrieNode::LeafNode { value, .. } | TrieNode::HybridNode { value, .. } => Ok(value),
            TrieNode::RootNode { .. } | TrieNode::BranchNode { .. } => {
                error!(
                    "Node with key {} is a branch node and so it can't have a value!",
                    self.key()
                );
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
            TrieNode::LeafNode { key, value } => TrieNode::HybridNode {
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
            TrieNode::HybridNode {
                key,
                value,
                children,
            } if children.iter().all(|child| child.is_none()) => TrieNode::LeafNode { key, value },
            other => other,
        };

        Ok(self)
    }

    /// Adds, or overwrites, the value at the given node. If successful, it returns  the modified
    /// node.
    pub fn put_value<T: Serialize>(mut self, new_value: T) -> Result<Self, MerkleRadixTrieError> {
        // Turn branch node into a hybrid node.
        self = match self {
            TrieNode::BranchNode { key, children } => TrieNode::HybridNode {
                key,
                value: new_value.serialize_to_vec(),
                children,
            },
            TrieNode::RootNode { .. } => return Err(MerkleRadixTrieError::RootCantHaveValue),
            other => other,
        };

        new_value.serialize(self.raw_value_mut()?)?;

        Ok(self)
    }

    /// Removes the value at the given node. Returns the modified node or None if the node is a
    /// leaf node.
    pub fn remove_value(self) -> Option<Self> {
        match self {
            TrieNode::LeafNode { .. } => None,
            TrieNode::HybridNode { key, children, .. } => {
                // Turn this hybrid node into a branch node.
                Some(TrieNode::BranchNode { key, children })
            }
            // TODO Error instead silently doing nothing?
            TrieNode::BranchNode { .. } | TrieNode::RootNode { .. } => Some(self),
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
    fn remove_child_works() {
        let key: KeyNibbles = "cfb986".parse().unwrap();

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

        branch_node = branch_node.remove_child(&child_key_1).unwrap();
        assert_eq!(
            branch_node.get_child_hash(&child_key_1),
            Err(MerkleRadixTrieError::ChildDoesNotExist)
        );

        branch_node = branch_node.remove_child(&child_key_2).unwrap();
        assert_eq!(
            branch_node.get_child_hash(&child_key_2),
            Err(MerkleRadixTrieError::ChildDoesNotExist)
        );

        branch_node = branch_node.remove_child(&child_key_3).unwrap();
        assert_eq!(
            branch_node.get_child_hash(&child_key_3),
            Err(MerkleRadixTrieError::ChildDoesNotExist)
        );

        branch_node = branch_node.remove_child(&child_key_4).unwrap();
        assert_eq!(
            branch_node.get_child_hash(&child_key_4),
            Err(MerkleRadixTrieError::ChildDoesNotExist)
        );
    }
}
