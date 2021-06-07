use std::io;
use std::iter;
use std::slice;

use beserial::{Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};
use nimiq_database::{FromDatabaseValue, IntoDatabaseValue};
use nimiq_hash::{Blake2bHash, Hash, SerializeContent};

use crate::error::MerkleRadixTrieError;
use crate::key_nibbles::KeyNibbles;

/// A struct representing a node in the Merkle Radix Trie. It can be either a branch node, which has
/// only references to its children, or a leaf node, which contains a value. A branch node can have
/// up to 16 children, since we represent the keys in hexadecimal form, each child represents a
/// different hexadecimal character.
#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Debug)]
pub enum TrieNode<A: Serialize + Deserialize + Clone> {
    BranchNode {
        key: KeyNibbles,
        children: Box<[Option<TrieNodeChild>; 16]>,
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
    LeafNode = 0xff,
}

/// A struct representing the child of a node. It just contains the child's suffix (the part of the
/// child's key that is different from its parent) and its hash.
#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Debug, Serialize, Deserialize)]
pub struct TrieNodeChild {
    pub suffix: KeyNibbles,
    pub hash: Blake2bHash,
}

pub const NO_CHILDREN: [Option<TrieNodeChild>; 16] = [
    None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None,
];

impl<A: Serialize + Deserialize + Clone> TrieNode<A> {
    /// Creates a new leaf node.
    pub fn new_leaf(key: KeyNibbles, value: A) -> Self {
        TrieNode::LeafNode { key, value }
    }

    /// Creates a new empty branch node.
    pub fn new_branch(key: KeyNibbles) -> Self {
        TrieNode::BranchNode {
            key,
            children: Box::new(NO_CHILDREN),
        }
    }

    /// Checks if the node is a leaf node.
    pub fn is_leaf(&self) -> bool {
        match self {
            TrieNode::BranchNode { .. } => false,
            TrieNode::LeafNode { .. } => true,
        }
    }

    /// Checks if the node is a branch node.
    pub fn is_branch(&self) -> bool {
        match self {
            TrieNode::LeafNode { .. } => false,
            TrieNode::BranchNode { .. } => true,
        }
    }

    /// Returns the type of a node.
    pub fn ty(&self) -> TrieNodeType {
        match self {
            TrieNode::LeafNode { .. } => TrieNodeType::LeafNode,
            TrieNode::BranchNode { .. } => TrieNodeType::BranchNode,
        }
    }

    /// Returns the key of a node.
    pub fn key(&self) -> &KeyNibbles {
        match self {
            TrieNode::LeafNode { ref key, .. } => &key,
            TrieNode::BranchNode { ref key, .. } => &key,
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
            info!(
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
        match self {
            TrieNode::LeafNode { .. } => Err(MerkleRadixTrieError::LeavesHaveNoChildren),
            TrieNode::BranchNode { children, .. } => {
                if let Some(child) = &children[self.get_child_index(child_prefix)?] {
                    return Ok(&child.hash);
                }
                Err(MerkleRadixTrieError::ChildDoesNotExist)
            }
        }
    }

    /// Returns the key of the current node's child with the given prefix.
    pub fn get_child_key(
        &self,
        child_prefix: &KeyNibbles,
    ) -> Result<KeyNibbles, MerkleRadixTrieError> {
        match self {
            TrieNode::LeafNode { .. } => Err(MerkleRadixTrieError::LeavesHaveNoChildren),
            TrieNode::BranchNode { ref children, .. } => {
                if let Some(ref child) = children[self.get_child_index(child_prefix)?] {
                    return Ok(self.key() + &child.suffix);
                }
                Err(MerkleRadixTrieError::ChildDoesNotExist)
            }
        }
    }

    /// Add a child, with the given key and hash, to the current node. If successful, it returns the
    /// modified node.
    pub fn put_child(
        mut self,
        child_key: &KeyNibbles,
        child_hash: Blake2bHash,
    ) -> Result<Self, MerkleRadixTrieError> {
        let child_index = self.get_child_index(&child_key)?;

        let suffix = child_key.suffix(self.key().len() as u8);

        match self {
            TrieNode::LeafNode { .. } => {
                return Err(MerkleRadixTrieError::LeavesHaveNoChildren);
            }
            TrieNode::BranchNode {
                ref mut children, ..
            } => {
                children[child_index] = Some(TrieNodeChild {
                    suffix,
                    hash: child_hash,
                });
            }
        };

        Ok(self)
    }

    /// Removes a child, with the given key, to the current node. If successful, it returns the
    /// modified node.
    pub fn remove_child(mut self, child_key: &KeyNibbles) -> Result<Self, MerkleRadixTrieError> {
        let child_index = self.get_child_index(child_key)?;

        match self {
            TrieNode::LeafNode { .. } => {
                return Err(MerkleRadixTrieError::LeavesHaveNoChildren);
            }
            TrieNode::BranchNode {
                ref mut children, ..
            } => {
                children[child_index] = None;
            }
        };

        Ok(self)
    }

    /// Adds, or overwrites, the value at the given node. If successful, it returns the modified
    /// node.
    pub fn put_value(mut self, new_value: A) -> Result<Self, MerkleRadixTrieError> {
        match &mut self {
            TrieNode::LeafNode { ref mut value, .. } => {
                *value = new_value;
            }
            TrieNode::BranchNode { .. } => {
                return Err(MerkleRadixTrieError::BranchesHaveNoValue);
            }
        };

        Ok(self)
    }

    pub fn iter_children(&self) -> Iter {
        self.into_iter()
    }

    pub fn iter_children_mut(&mut self) -> IterMut {
        self.into_iter()
    }
}

impl<A: Serialize + Deserialize + Clone> Serialize for TrieNode<A> {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size: usize = 0;
        size += Serialize::serialize(&self.ty(), writer)?;
        size += Serialize::serialize(self.key(), writer)?;

        match self {
            TrieNode::LeafNode { ref value, .. } => {
                size += Serialize::serialize(value, writer)?;
            }
            TrieNode::BranchNode { ref children, .. } => {
                let child_count: u8 = children
                    .iter()
                    .fold(0, |acc, child| acc + if child.is_none() { 0 } else { 1 });
                Serialize::serialize(&child_count, writer)?;

                for child in children.iter() {
                    if let Some(ref child) = child {
                        size += Serialize::serialize(&child, writer)?;
                    }
                }
            }
        }

        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size = /*type*/ 1;

        size += Serialize::serialized_size(self.key());

        match self {
            TrieNode::LeafNode { ref value, .. } => {
                size += Serialize::serialized_size(value);
            }
            TrieNode::BranchNode { ref children, .. } => {
                size += /*count*/ 1;

                for child in children.iter() {
                    if let Some(ref child) = child {
                        size += Serialize::serialized_size(&child);
                    }
                }
            }
        }

        size
    }
}

impl<A: Serialize + Deserialize + Clone> Deserialize for TrieNode<A> {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let node_type: TrieNodeType = Deserialize::deserialize(reader)?;

        let prefix: KeyNibbles = Deserialize::deserialize(reader)?;

        match node_type {
            TrieNodeType::LeafNode => {
                let value: A = Deserialize::deserialize(reader)?;

                Ok(TrieNode::new_leaf(prefix, value))
            }
            TrieNodeType::BranchNode => {
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

                Ok(TrieNode::BranchNode {
                    key: prefix,
                    children: Box::new(children),
                })
            }
        }
    }
}

impl<A: Serialize + Deserialize + Clone> SerializeContent for TrieNode<A> {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> {
        Ok(self.serialize(writer)?)
    }
}

impl<A: Serialize + Deserialize + Clone> Hash for TrieNode<A> {}

impl<A: Serialize + Deserialize + Clone> IntoDatabaseValue for TrieNode<A> {
    fn database_byte_size(&self) -> usize {
        self.serialized_size()
    }

    fn copy_into_database(&self, mut bytes: &mut [u8]) {
        Serialize::serialize(&self, &mut bytes).unwrap();
    }
}

impl<A: Serialize + Deserialize + Clone> FromDatabaseValue for TrieNode<A> {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self>
    where
        Self: Sized,
    {
        let mut cursor = io::Cursor::new(bytes);
        Ok(Deserialize::deserialize(&mut cursor)?)
    }
}

#[allow(clippy::type_complexity)]
type AccountsTreeNodeIter<'a> = Option<
    iter::FilterMap<
        slice::Iter<'a, Option<TrieNodeChild>>,
        fn(&Option<TrieNodeChild>) -> Option<&TrieNodeChild>,
    >,
>;

pub struct Iter<'a> {
    it: AccountsTreeNodeIter<'a>,
}

impl<'a> iter::Iterator for Iter<'a> {
    type Item = &'a TrieNodeChild;

    fn next(&mut self) -> Option<&'a TrieNodeChild> {
        if let Some(ref mut it) = self.it {
            return it.next();
        }

        None
    }
}

impl<'a, A: Serialize + Deserialize + Clone> iter::IntoIterator for &'a TrieNode<A> {
    type Item = &'a TrieNodeChild;
    type IntoIter = Iter<'a>;

    fn into_iter(self) -> Iter<'a> {
        match self {
            TrieNode::LeafNode { .. } => Iter { it: None },
            TrieNode::BranchNode { ref children, .. } => Iter {
                it: Some(children.iter().filter_map(Option::as_ref)),
            },
        }
    }
}

type AccountsTreeNodeChildFilterMap<'a> = iter::FilterMap<
    slice::IterMut<'a, Option<TrieNodeChild>>,
    fn(&mut Option<TrieNodeChild>) -> Option<&mut TrieNodeChild>,
>;

pub struct IterMut<'a> {
    it: Option<AccountsTreeNodeChildFilterMap<'a>>,
}

impl<'a> iter::Iterator for IterMut<'a> {
    type Item = &'a mut TrieNodeChild;

    fn next(&mut self) -> Option<&'a mut TrieNodeChild> {
        if let Some(ref mut it) = self.it {
            return it.next();
        }
        None
    }
}

impl<'a, A: Serialize + Deserialize + Clone> iter::IntoIterator for &'a mut TrieNode<A> {
    type Item = &'a mut TrieNodeChild;
    type IntoIter = IterMut<'a>;

    fn into_iter(self) -> IterMut<'a> {
        match self {
            TrieNode::LeafNode { .. } => IterMut { it: None },
            TrieNode::BranchNode {
                ref mut children, ..
            } => IterMut {
                it: Some(children.iter_mut().filter_map(Option::as_mut)),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            Ok(child_key_1.clone())
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
