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
/// only references to its children, or a leaf node, which contains a value. A branch node can have
/// up to 16 children, since we represent the keys in hexadecimal form, each child represents a
/// different hexadecimal character.
#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Debug)]
pub enum TrieNode<A: Serialize + Deserialize> {
    RootNode {
        children: TrieNodeChildren,
        num_leaves: u64,
    },
    BranchNode {
        key: KeyNibbles,
        children: TrieNodeChildren,
    },
    LeafNode {
        key: KeyNibbles,
        value: A,
    },
}

/// Just a enum stating the trie node type, branch or leaf. It is used to serialize/deserialize a node.
#[derive(PartialEq, Eq, PartialOrd, Ord, Copy, Clone, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum TrieNodeType {
    BranchNode = 0x00,
    RootNode = 0x01,
    LeafNode = 0xff,
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

impl<A: Serialize + Deserialize> TrieNode<A> {
    /// Creates a new leaf node.
    pub fn new_leaf(key: KeyNibbles, value: A) -> Self {
        TrieNode::LeafNode { key, value }
    }

    /// Creates an empty root node
    pub fn new_root() -> Self {
        TrieNode::RootNode {
            children: NO_CHILDREN,
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

    /// Checks if the node is a leaf node.
    pub fn is_leaf(&self) -> bool {
        match self {
            TrieNode::LeafNode { .. } => true,
            TrieNode::RootNode { .. } => false,
            TrieNode::BranchNode { .. } => false,
        }
    }

    /// Checks if the node is a branch node.
    pub fn is_branch(&self) -> bool {
        !self.is_leaf()
    }

    /// Returns the type of a node.
    pub fn ty(&self) -> TrieNodeType {
        match self {
            TrieNode::LeafNode { .. } => TrieNodeType::LeafNode,
            TrieNode::RootNode { .. } => TrieNodeType::RootNode,
            TrieNode::BranchNode { .. } => TrieNodeType::BranchNode,
        }
    }

    /// Returns the key of a node.
    pub fn key(&self) -> &KeyNibbles {
        match self {
            TrieNode::LeafNode { ref key, .. } => key,
            TrieNode::RootNode { .. } => &KeyNibbles::ROOT,
            TrieNode::BranchNode { ref key, .. } => key,
        }
    }

    pub fn children(&self) -> Result<&[Option<TrieNodeChild>; 16], MerkleRadixTrieError> {
        match self {
            TrieNode::RootNode { ref children, .. } | TrieNode::BranchNode { ref children, .. } => {
                Ok(&children.0)
            }
            TrieNode::LeafNode { .. } => Err(MerkleRadixTrieError::LeavesHaveNoChildren),
        }
    }

    pub fn children_mut(
        &mut self,
    ) -> Result<&mut [Option<TrieNodeChild>; 16], MerkleRadixTrieError> {
        match self {
            TrieNode::RootNode {
                ref mut children, ..
            }
            | TrieNode::BranchNode {
                ref mut children, ..
            } => Ok(&mut children.0),
            TrieNode::LeafNode { .. } => Err(MerkleRadixTrieError::LeavesHaveNoChildren),
        }
    }

    /// Returns the value of a node, if it is a leaf node.
    pub fn value(&self) -> Result<&A, MerkleRadixTrieError> {
        match self {
            TrieNode::LeafNode { ref value, .. } => Ok(value),
            TrieNode::RootNode { .. } | TrieNode::BranchNode { .. } => {
                error!(
                    "Node with key {} is a branch node and so it can't have a value!",
                    self.key()
                );

                Err(MerkleRadixTrieError::BranchesHaveNoValue)
            }
        }
    }

    /// Returns the value of a node, if it is a leaf node.
    pub fn value_mut(&mut self) -> Result<&mut A, MerkleRadixTrieError> {
        match self {
            TrieNode::LeafNode { ref mut value, .. } => Ok(value),
            TrieNode::RootNode { .. } | TrieNode::BranchNode { .. } => {
                error!(
                    "Node with key {} is a branch node and so it can't have a value!",
                    self.key()
                );

                Err(MerkleRadixTrieError::BranchesHaveNoValue)
            }
        }
    }

    /// Returns the value of a node, if it is a leaf node.
    pub fn into_value(self) -> Result<A, MerkleRadixTrieError> {
        match self {
            TrieNode::LeafNode { value, .. } => Ok(value),
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

    /// Add a child, with the given key and hash, to the current node. If successful, it returns the
    /// modified node.
    pub fn put_child(
        mut self,
        child_key: &KeyNibbles,
        child_hash: Blake2bHash,
    ) -> Result<Self, MerkleRadixTrieError> {
        let child_index = self.get_child_index(child_key)?;
        let suffix = child_key.suffix(self.key().len() as u8);
        self.children_mut()?[child_index] = Some(TrieNodeChild {
            suffix,
            hash: child_hash,
        });
        Ok(self)
    }

    /// Removes a child, with the given key, to the current node. If successful, it returns the
    /// modified node.
    pub fn remove_child(mut self, child_key: &KeyNibbles) -> Result<Self, MerkleRadixTrieError> {
        let child_index = self.get_child_index(child_key)?;
        self.children_mut()?[child_index] = None;
        Ok(self)
    }

    /// Adds, or overwrites, the value at the given node. If successful, it returns the modified
    /// node.
    pub fn put_value(mut self, new_value: A) -> Result<Self, MerkleRadixTrieError> {
        *self.value_mut()? = new_value;
        Ok(self)
    }

    pub fn iter_children(&self) -> Iter {
        self.into_iter()
    }

    pub fn iter_children_mut(&mut self) -> IterMut {
        self.into_iter()
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

impl<A: Serialize + Deserialize> Serialize for TrieNode<A> {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size: usize = 0;
        size += Serialize::serialize(&self.ty(), writer)?;

        match self {
            TrieNode::LeafNode { ref key, ref value } => {
                size += Serialize::serialize(key, writer)?;
                size += Serialize::serialize(value, writer)?;
            }
            TrieNode::RootNode {
                ref children,
                num_leaves,
            } => {
                size += Serialize::serialize(children, writer)?;
                size += Serialize::serialize(num_leaves, writer)?;
            }
            TrieNode::BranchNode {
                ref key,
                ref children,
            } => {
                size += Serialize::serialize(key, writer)?;
                size += Serialize::serialize(children, writer)?;
            }
        }

        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size = /*type*/ 1;

        match self {
            TrieNode::LeafNode { ref key, ref value } => {
                size += Serialize::serialized_size(key);
                size += Serialize::serialized_size(value);
            }
            TrieNode::RootNode {
                ref children,
                num_leaves,
            } => {
                size += Serialize::serialized_size(children);
                size += Serialize::serialized_size(num_leaves);
            }
            TrieNode::BranchNode {
                ref key,
                ref children,
            } => {
                size += Serialize::serialized_size(key);
                size += Serialize::serialized_size(children);
            }
        }

        size
    }
}

impl<A: Serialize + Deserialize> Deserialize for TrieNode<A> {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let node_type: TrieNodeType = Deserialize::deserialize(reader)?;

        Ok(match node_type {
            TrieNodeType::LeafNode => {
                let key: KeyNibbles = Deserialize::deserialize(reader)?;
                let value: A = Deserialize::deserialize(reader)?;
                TrieNode::LeafNode { key, value }
            }
            TrieNodeType::RootNode => {
                let children: TrieNodeChildren = Deserialize::deserialize(reader)?;
                let num_leaves: u64 = Deserialize::deserialize(reader)?;
                TrieNode::RootNode {
                    children,
                    num_leaves,
                }
            }
            TrieNodeType::BranchNode => {
                let key: KeyNibbles = Deserialize::deserialize(reader)?;
                let children: TrieNodeChildren = Deserialize::deserialize(reader)?;
                TrieNode::BranchNode { key, children }
            }
        })
    }
}

impl<A: Serialize + Deserialize> SerializeContent for TrieNode<A> {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> {
        Ok(self.serialize(writer)?)
    }
}

impl<A: Serialize + Deserialize> Hash for TrieNode<A> {}

impl<A: Serialize + Deserialize> IntoDatabaseValue for TrieNode<A> {
    fn database_byte_size(&self) -> usize {
        self.serialized_size()
    }

    fn copy_into_database(&self, mut bytes: &mut [u8]) {
        Serialize::serialize(&self, &mut bytes).unwrap();
    }
}

impl<A: Serialize + Deserialize> FromDatabaseValue for TrieNode<A> {
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

impl<'a> iter::Iterator for Iter<'a> {
    type Item = &'a TrieNodeChild;

    fn next(&mut self) -> Option<&'a TrieNodeChild> {
        self.it.next()
    }
}

impl<'a, A: Serialize + Deserialize> iter::IntoIterator for &'a TrieNode<A> {
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

impl<'a> iter::Iterator for IterMut<'a> {
    type Item = &'a mut TrieNodeChild;

    fn next(&mut self) -> Option<&'a mut TrieNodeChild> {
        self.it.next()
    }
}

impl<'a, A: Serialize + Deserialize> iter::IntoIterator for &'a mut TrieNode<A> {
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
    use super::*;
    use nimiq_test_log::test;

    #[test]
    fn get_child_index_works() {
        let key: KeyNibbles = "cfb986".parse().unwrap();

        let leaf_node = TrieNode::<u32>::new_leaf(key.clone(), 666);
        let branch_node = TrieNode::<u32>::new_branch(key);

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

        let leaf_node = TrieNode::<u32>::new_leaf(key.clone(), 666);
        let mut branch_node = TrieNode::<u32>::new_branch(key);

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

        let leaf_node = TrieNode::<u32>::new_leaf(key.clone(), 666);
        let mut branch_node = TrieNode::<u32>::new_branch(key);

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

        let mut branch_node = TrieNode::<u32>::new_branch(key);

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
