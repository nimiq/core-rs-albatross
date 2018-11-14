use super::AddressNibbles;
use super::super::Account;
use beserial::{Serialize, SerializingError, Deserialize, WriteBytesExt, ReadBytesExt};
use crate::consensus::base::primitive::hash::{Hash, Blake2bHash, SerializeContent};
use std::io;
use std::iter;
use std::slice;
use crate::utils::db::{FromDatabaseValue, IntoDatabaseValue};

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Debug, Serialize, Deserialize)]
pub (in super) struct AccountsTreeNodeChild {
    pub (in super) suffix: AddressNibbles,
    pub (in super) hash: Blake2bHash,
}

pub (in super) const NO_CHILDREN: [Option<AccountsTreeNodeChild>; 16] = [None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None];

#[derive(PartialEq, Eq, PartialOrd, Ord, Copy, Clone, Debug, Serialize, Deserialize)]
#[repr(u8)]
enum AccountsTreeNodeType {
    BranchNode = 0x00,
    TerminalNode = 0xff,
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Debug)]
pub (in super) enum AccountsTreeNode {
    BranchNode {
        prefix: AddressNibbles,
        children: [Option<AccountsTreeNodeChild>; 16],
    },
    TerminalNode {
        prefix: AddressNibbles,
        account: Account,
    }
}

impl AccountsTreeNode {
    pub (in super) fn new_terminal(prefix: AddressNibbles, account: Account) -> Self {
        return AccountsTreeNode::TerminalNode { prefix, account };
    }

    pub (in super) fn new_branch(prefix: AddressNibbles, children: [Option<AccountsTreeNodeChild>; 16]) -> Self {
        return AccountsTreeNode::BranchNode { prefix, children };
    }

    #[inline]
    pub (in super) fn is_terminal(&self) -> bool {
        return match *self {
            AccountsTreeNode::TerminalNode { .. } => true,
            AccountsTreeNode::BranchNode { .. } => false,
        };
    }

    #[inline]
    pub (in super) fn is_branch(&self) -> bool {
        return match *self {
            AccountsTreeNode::TerminalNode { .. } => false,
            AccountsTreeNode::BranchNode { .. } => true,
        };
    }

    pub (in super) fn get_child_hash(&self, prefix: &AddressNibbles) -> Option<&Blake2bHash> {
        match *self {
            AccountsTreeNode::TerminalNode { .. } => return None,
            AccountsTreeNode::BranchNode { ref children, .. } => {
                if let Some(ref child) = children[self.get_child_index(prefix)?] {
                    return Some(&child.hash);
                }
                return None;
            },
        };
    }

    pub (in super) fn get_child_prefix(&self, prefix: &AddressNibbles) -> Option<AddressNibbles> {
        match *self {
            AccountsTreeNode::TerminalNode { .. } => return None,
            AccountsTreeNode::BranchNode { ref children, .. } => {
                if let Some(ref child) = children[self.get_child_index(prefix)?] {
                    return Some(self.prefix() + &child.suffix);
                }
                return None;
            },
        };
    }

    #[inline]
    pub (in super) fn prefix(&self) -> &AddressNibbles {
        return match *self {
            AccountsTreeNode::TerminalNode { ref prefix, .. } => &prefix,
            AccountsTreeNode::BranchNode { ref prefix, .. } => &prefix,
        };
    }

    #[inline]
    fn node_type(&self) -> AccountsTreeNodeType {
        return match *self {
            AccountsTreeNode::TerminalNode { .. } => AccountsTreeNodeType::TerminalNode,
            AccountsTreeNode::BranchNode { .. } => AccountsTreeNodeType::BranchNode,
        };
    }

    #[inline]
    pub (in super) fn get_child_index(&self, prefix: &AddressNibbles) -> Option<usize> {
        assert!(self.prefix().is_prefix_of(prefix), "prefix is not a child of the current node");
        return prefix.get(self.prefix().len());
    }

    pub (in super) fn with_child(mut self, prefix: &AddressNibbles, hash: Blake2bHash) -> Option<Self> {
        let child_index = self.get_child_index(&prefix)?;
        let suffix = prefix.suffix(self.prefix().len() as u8);
        match self {
            AccountsTreeNode::TerminalNode { .. } => { return None; },
            AccountsTreeNode::BranchNode { ref mut children, .. } => {
                let child = AccountsTreeNodeChild { suffix, hash };
                children[child_index] = Some(child);
            },
        };
        return Some(self);
    }

    pub (in super) fn without_child(mut self, suffix: AddressNibbles) -> Option<Self> {
        let child_index = self.get_child_index(&suffix)?;
        match self {
            AccountsTreeNode::TerminalNode { .. } => { return None; },
            AccountsTreeNode::BranchNode { ref mut children, .. } => {
                children[child_index] = None;
            },
        };
        return Some(self);
    }

    pub (in super) fn with_account(mut self, new_account: Account) -> Option<Self> {
        match &mut self {
            AccountsTreeNode::TerminalNode { ref mut account, .. } => {
                *account = new_account;
            },
            AccountsTreeNode::BranchNode { .. } => { return None; },
        };
        return Some(self);
    }

    pub (in super) fn iter_children(&self) -> Iter {
        return self.into_iter();
    }

    pub (in super) fn iter_children_mut(&mut self) -> IterMut {
        return self.into_iter();
    }
}

impl Serialize for AccountsTreeNode {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size: usize = 0;
        size += Serialize::serialize(&self.node_type(), writer)?;
        size += Serialize::serialize(self.prefix(), writer)?;

        match *self {
            AccountsTreeNode::TerminalNode { ref account, .. } => {
                size += Serialize::serialize(&account, writer)?;
            },
            AccountsTreeNode::BranchNode { ref children, .. } => {
                let child_count: u8 = children.iter().fold(0, |acc, child| { acc + if child.is_none() { 0 } else { 1 } });
                Serialize::serialize(&child_count, writer)?;
                for child in children.iter() {
                    if let Some(ref child) = child {
                        size += Serialize::serialize(&child, writer)?;
                    }
                }
            },
        }

        return Ok(size);
    }

    fn serialized_size(&self) -> usize {
        let mut size = /*type*/ 1;
        size += Serialize::serialized_size(self.prefix());

        match *self {
            AccountsTreeNode::TerminalNode { ref account, .. } => {
                size += Serialize::serialized_size(&account);
            },
            AccountsTreeNode::BranchNode { ref children, .. } => {
                size += /*count*/ 1;
                for child in children.iter() {
                    if let Some(ref child) = child {
                        size += Serialize::serialized_size(&child);
                    }
                }
            },
        }

        return size;
    }
}

impl Deserialize for AccountsTreeNode {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let node_type: AccountsTreeNodeType = Deserialize::deserialize(reader)?;
        let prefix: AddressNibbles = Deserialize::deserialize(reader)?;

        match node_type {
            AccountsTreeNodeType::TerminalNode => {
                let account: Account = Deserialize::deserialize(reader)?;
                return Ok(AccountsTreeNode::new_terminal(prefix, account));
            },
            AccountsTreeNodeType::BranchNode => {
                let child_count: u8 = Deserialize::deserialize(reader)?;
                let mut children = NO_CHILDREN;
                for i in 0..child_count {
                    let child: AccountsTreeNodeChild = Deserialize::deserialize(reader)?;
                    if let Some(i) = child.suffix.get(0) {
                        children[i] = Some(child);
                    } else {
                        return Err(io::Error::from(io::ErrorKind::InvalidData).into());
                    }
                }
                return Ok(AccountsTreeNode::new_branch(prefix, children));
            },
        }
    }
}

impl SerializeContent for AccountsTreeNode {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> { Ok(self.serialize(writer)?) }
}

impl IntoDatabaseValue for AccountsTreeNode {
    fn database_byte_size(&self) -> usize {
        return self.serialized_size();
    }

    fn copy_into_database(&self, mut bytes: &mut [u8]) {
        Serialize::serialize(&self, &mut bytes).unwrap();
    }
}

impl FromDatabaseValue for AccountsTreeNode {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self> where Self: Sized {
        let mut cursor = io::Cursor::new(bytes);
        return Ok(Deserialize::deserialize(&mut cursor)?);
    }
}

impl Hash for AccountsTreeNode {}

pub (in super) struct Iter<'a> {
    it: Option<iter::FilterMap<slice::Iter<'a, Option<AccountsTreeNodeChild>>, fn(&Option<AccountsTreeNodeChild>) -> Option<&AccountsTreeNodeChild>>>,
}

impl<'a> iter::Iterator for Iter<'a> {
    type Item = &'a AccountsTreeNodeChild;

    fn next(&mut self) -> Option<&'a AccountsTreeNodeChild> {
        if let Some(ref mut it) = self.it {
            return it.next();
        }
        return None;
    }
}

impl<'a> iter::IntoIterator for &'a AccountsTreeNode {
    type Item = &'a AccountsTreeNodeChild;
    type IntoIter = Iter<'a>;

    fn into_iter(self) -> Iter<'a> {
        return match self {
            AccountsTreeNode::TerminalNode { .. } => Iter { it: None },
            AccountsTreeNode::BranchNode { ref children, .. } => Iter { it: Some(children.iter().filter_map(|o| o.as_ref())) },
        };
    }
}

pub (in super) struct IterMut<'a> {
    it: Option<iter::FilterMap<slice::IterMut<'a, Option<AccountsTreeNodeChild>>, fn(&mut Option<AccountsTreeNodeChild>) -> Option<&mut AccountsTreeNodeChild>>>,
}

impl<'a> iter::Iterator for IterMut<'a> {
    type Item = &'a mut AccountsTreeNodeChild;

    fn next(&mut self) -> Option<&'a mut AccountsTreeNodeChild> {
        if let Some(ref mut it) = self.it {
            return it.next();
        }
        return None;
    }
}

impl<'a> iter::IntoIterator for &'a mut AccountsTreeNode {
    type Item = &'a mut AccountsTreeNodeChild;
    type IntoIter = IterMut<'a>;

    fn into_iter(self) -> IterMut<'a> {
        return match self {
            AccountsTreeNode::TerminalNode { .. } => IterMut { it: None },
            AccountsTreeNode::BranchNode { ref mut children, .. } => IterMut { it: Some(children.iter_mut().filter_map(|o| o.as_mut())) },
        };
    }
}
