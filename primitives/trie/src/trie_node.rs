use std::io;
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
pub struct TrieNode {
    pub key: KeyNibbles,
    pub root_data: Option<RootData>,
    #[beserial(len_type(u16))]
    pub serialized_value: Option<Vec<u8>>,
    pub children: [Option<TrieNodeChild>; 16],
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct RootData {
    pub num_branches: u64,
    pub num_hybrids: u64,
    pub num_leaves: u64,
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

pub enum TrieNodeKind {
    Root,
    Branch,
    Hybrid,
    Leaf,
}

impl TrieNode {
    /// Creates a new leaf node.
    pub fn new_leaf<T: Serialize>(key: KeyNibbles, value: T) -> Self {
        TrieNode {
            key,
            root_data: None,
            serialized_value: Some(value.serialize_to_vec()),
            children: NO_CHILDREN,
        }
    }

    /// Creates an empty root node
    pub fn new_root() -> Self {
        TrieNode {
            key: KeyNibbles::ROOT,
            root_data: Some(RootData {
                num_branches: 0,
                num_hybrids: 0,
                num_leaves: 0,
            }),
            serialized_value: None,
            children: NO_CHILDREN,
        }
    }

    /// Creates a new empty branch node.
    pub fn new_empty(key: KeyNibbles) -> Self {
        TrieNode {
            key,
            root_data: None,
            serialized_value: None,
            children: NO_CHILDREN,
        }
    }

    pub fn is_empty(&self) -> bool {
        !self.is_root() && !self.has_value() && !self.has_children()
    }

    pub fn is_root(&self) -> bool {
        self.root_data.is_some()
    }

    pub fn has_value(&self) -> bool {
        self.serialized_value.is_some()
    }

    pub fn has_children(&self) -> bool {
        self.iter_children().count() != 0
    }

    pub fn kind(&self) -> Option<TrieNodeKind> {
        Some(
            match (self.is_root(), self.has_children(), self.has_value()) {
                (false, false, false) => return None,
                (false, false, true) => TrieNodeKind::Leaf,
                (false, true, false) => TrieNodeKind::Branch,
                (false, true, true) => TrieNodeKind::Hybrid,
                (true, _, _) => TrieNodeKind::Root,
            },
        )
    }

    /// Returns the value of a node, if it is a leaf or hybrid node.
    pub fn value<T: Deserialize>(&self) -> Result<T, MerkleRadixTrieError> {
        self.serialized_value
            .as_ref()
            .map(|v| Deserialize::deserialize(&mut &v[..]).unwrap())
            .ok_or(MerkleRadixTrieError::BranchesHaveNoValue)
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

        // Key length has to be smaller or equal to the child prefix length, so this should never panic!
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

    pub fn child_key(&self, child_prefix: &KeyNibbles) -> Result<KeyNibbles, MerkleRadixTrieError> {
        Ok(&self.key + &self.child(child_prefix)?.suffix)
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

    pub fn put_value<T: Serialize>(&mut self, new_value: T) -> Result<(), MerkleRadixTrieError> {
        if self.root_data.is_some() {
            return Err(MerkleRadixTrieError::RootCantHaveValue);
        }
        self.serialized_value = Some(new_value.serialize_to_vec());
        Ok(())
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
        let mut result = 0;
        result += self.key.serialize(writer)?;
        if let Some(v) = &self.serialized_value {
            writer.write_u8(1)?;
            result += v.serialize::<u16, _>(writer)?;
        } else {
            writer.write_u8(0)?;
        }
        result += self.children.serialize(writer)?;
        Ok(result)
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
    use nimiq_test_log::test;

    use super::*;

    #[test]
    fn child_index_works() {
        let key: KeyNibbles = "cfb986".parse().unwrap();

        let leaf_node = TrieNode::new_leaf::<u32>(key.clone(), 666);
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

        let leaf_node = TrieNode::new_leaf::<u32>(key.clone(), 666);
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

        let leaf_node = TrieNode::new_leaf::<u32>(key.clone(), 666);
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

        assert_eq!(branch_node.child_key(&child_key_1), Ok(child_key_1.clone()));
        assert_eq!(branch_node.child_key(&child_key_2), Ok(child_key_2.clone()));
        assert_eq!(branch_node.child_key(&child_key_3), Ok(child_key_3.clone()));
        assert_eq!(branch_node.child_key(&child_key_4), Ok(child_key_4.clone()));

        assert_eq!(
            branch_node.child_key(&child_key_1.slice(0, 7)),
            Ok(child_key_1)
        );

        assert_eq!(
            leaf_node.child_key(&child_key_2),
            Err(MerkleRadixTrieError::ChildDoesNotExist)
        );

        let child_key_5 = "c0b986d50".parse().unwrap();
        assert_eq!(
            branch_node.child_key(&child_key_5),
            Err(MerkleRadixTrieError::WrongPrefix)
        );
    }

    #[test]
    fn put_remove_child_works() {
        let key: KeyNibbles = "cfb986".parse().unwrap();

        let mut node = TrieNode::new_leaf(key, 666u32);

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

        assert_eq!(node.value(), Ok(666u32));
    }

    #[test]
    fn put_value_works() {
        let key: KeyNibbles = "cfb986".parse().unwrap();

        let mut leaf_node = TrieNode::new_leaf::<u32>(key.clone(), 666);
        assert_eq!(leaf_node.value(), Ok(666u32));
        leaf_node.put_value(999).unwrap();
        assert_eq!(leaf_node.value(), Ok(999u32));

        let mut hybrid_node = TrieNode::new_leaf::<u32>(key.clone(), 666);
        assert_eq!(hybrid_node.value(), Ok(666u32));
        hybrid_node.put_value(999).unwrap();
        assert_eq!(hybrid_node.value(), Ok(999u32));

        let mut branch_node = TrieNode::new_empty(key);
        assert_eq!(
            branch_node.value::<u32>(),
            Err(MerkleRadixTrieError::BranchesHaveNoValue)
        );
        branch_node.put_value(999).unwrap();
        assert_eq!(branch_node.value(), Ok(999u32));

        let mut root_node = TrieNode::new_root();
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

        let mut leaf_node = TrieNode::new_leaf::<u32>(key.clone(), 666);
        assert_eq!(leaf_node.value(), Ok(666u32));
        leaf_node.serialized_value = None;
        assert!(leaf_node.is_empty());

        let mut hybrid_node = TrieNode::new_leaf::<u32>(key.clone(), 666);
        assert_eq!(hybrid_node.value(), Ok(666u32));
        hybrid_node.serialized_value = None;
        assert_eq!(
            hybrid_node.value::<u32>(),
            Err(MerkleRadixTrieError::BranchesHaveNoValue)
        );

        let mut branch_node = TrieNode::new_empty(key);
        assert_eq!(
            branch_node.value::<u32>(),
            Err(MerkleRadixTrieError::BranchesHaveNoValue)
        );
        branch_node.serialized_value = None;
        assert_eq!(
            branch_node.value::<u32>(),
            Err(MerkleRadixTrieError::BranchesHaveNoValue)
        );

        let mut root_node = TrieNode::new_root();
        assert_eq!(
            root_node.value::<u32>(),
            Err(MerkleRadixTrieError::BranchesHaveNoValue)
        );
        root_node.serialized_value = None;
        assert_eq!(
            root_node.value::<u32>(),
            Err(MerkleRadixTrieError::BranchesHaveNoValue)
        );
    }
}
