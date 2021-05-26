use std::io;
use std::iter;
use std::slice;

use beserial::{Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};
use nimiq_database::{FromDatabaseValue, IntoDatabaseValue};
use nimiq_hash::{Blake2bHash, Hash, SerializeContent};

use crate::prefix_nibbles::PrefixNibbles;

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Debug)]
pub enum TrieNode<A: Serialize + Deserialize + Clone> {
    BranchNode {
        prefix: PrefixNibbles,
        children: Box<[Option<TrieNodeChild>; 16]>,
    },
    TerminalNode {
        prefix: PrefixNibbles,
        value: A,
    },
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Copy, Clone, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum TrieNodeType {
    BranchNode = 0x00,
    TerminalNode = 0xff,
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Debug, Serialize, Deserialize)]
pub struct TrieNodeChild {
    pub suffix: PrefixNibbles,
    pub hash: Blake2bHash,
}

pub const NO_CHILDREN: [Option<TrieNodeChild>; 16] = [
    None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None,
];

impl<A: Serialize + Deserialize + Clone> TrieNode<A> {
    pub fn new_terminal(prefix: PrefixNibbles, value: A) -> Self {
        TrieNode::TerminalNode { prefix, value }
    }

    pub fn new_branch(prefix: PrefixNibbles, children: [Option<TrieNodeChild>; 16]) -> Self {
        TrieNode::BranchNode {
            prefix,
            children: Box::new(children),
        }
    }

    pub fn is_terminal(&self) -> bool {
        match self {
            TrieNode::BranchNode { .. } => false,
            TrieNode::TerminalNode { .. } => true,
        }
    }

    pub fn is_branch(&self) -> bool {
        match self {
            TrieNode::TerminalNode { .. } => false,
            TrieNode::BranchNode { .. } => true,
        }
    }

    pub fn get_child_hash(&self, prefix: &PrefixNibbles) -> Option<&Blake2bHash> {
        match self {
            TrieNode::TerminalNode { .. } => None,
            TrieNode::BranchNode { children, .. } => {
                if let Some(child) = &children[self.get_child_index(prefix)?] {
                    return Some(&child.hash);
                }
                None
            }
        }
    }

    pub fn get_child_prefix(&self, prefix: &PrefixNibbles) -> Option<PrefixNibbles> {
        match self {
            TrieNode::TerminalNode { .. } => None,
            TrieNode::BranchNode { ref children, .. } => {
                if let Some(ref child) = children[self.get_child_index(prefix)?] {
                    return Some(self.prefix() + &child.suffix);
                }
                None
            }
        }
    }

    pub fn prefix(&self) -> &PrefixNibbles {
        match self {
            TrieNode::TerminalNode { ref prefix, .. } => &prefix,
            TrieNode::BranchNode { ref prefix, .. } => &prefix,
        }
    }

    fn ty(&self) -> TrieNodeType {
        match self {
            TrieNode::TerminalNode { .. } => TrieNodeType::TerminalNode,
            TrieNode::BranchNode { .. } => TrieNodeType::BranchNode,
        }
    }

    pub fn get_child_index(&self, prefix: &PrefixNibbles) -> Option<usize> {
        assert!(
            self.prefix().is_prefix_of(prefix),
            "prefix {} is not a child of the current node {}",
            prefix,
            self.prefix()
        );

        prefix.get(self.prefix().len())
    }

    pub fn put_child(mut self, prefix: &PrefixNibbles, hash: Blake2bHash) -> Option<Self> {
        let child_index = self.get_child_index(&prefix)?;

        let suffix = prefix.suffix(self.prefix().len() as u8);

        match self {
            TrieNode::TerminalNode { .. } => {
                return None;
            }
            TrieNode::BranchNode {
                ref mut children, ..
            } => {
                children[child_index] = Some(TrieNodeChild { suffix, hash });
            }
        };

        Some(self)
    }

    pub fn remove_child(mut self, suffix: &PrefixNibbles) -> Option<Self> {
        let child_index = self.get_child_index(suffix)?;

        match self {
            TrieNode::TerminalNode { .. } => {
                return None;
            }
            TrieNode::BranchNode {
                ref mut children, ..
            } => {
                children[child_index] = None;
            }
        };

        Some(self)
    }

    pub fn put_value(mut self, new_value: A) -> Option<Self> {
        match &mut self {
            TrieNode::TerminalNode { ref mut value, .. } => {
                *value = new_value;
            }
            TrieNode::BranchNode { .. } => {
                return None;
            }
        };

        Some(self)
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
        size += Serialize::serialize(self.prefix(), writer)?;

        match self {
            TrieNode::TerminalNode { ref value, .. } => {
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

        size += Serialize::serialized_size(self.prefix());

        match self {
            TrieNode::TerminalNode { ref value, .. } => {
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

        let prefix: PrefixNibbles = Deserialize::deserialize(reader)?;

        match node_type {
            TrieNodeType::TerminalNode => {
                let value: A = Deserialize::deserialize(reader)?;

                Ok(TrieNode::new_terminal(prefix, value))
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

                Ok(TrieNode::new_branch(prefix, children))
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
            TrieNode::TerminalNode { .. } => Iter { it: None },
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
            TrieNode::TerminalNode { .. } => IterMut { it: None },
            TrieNode::BranchNode {
                ref mut children, ..
            } => IterMut {
                it: Some(children.iter_mut().filter_map(Option::as_mut)),
            },
        }
    }
}
