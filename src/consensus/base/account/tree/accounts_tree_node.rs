use beserial::{Serialize, SerializingError, Deserialize, WriteBytesExt, ReadBytesExt};
use std::io;
use std::iter;
use std::slice;

use super::AddressNibbles;
use super::super::Account;
use crate::consensus::base::primitive::hash::{Hash, Blake2bHash, SerializeContent};
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
        assert!(self.prefix().is_prefix_of(prefix), "prefix {} is not a child of the current node {}", prefix, self.prefix());
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consensus::base::account::BasicAccount;

    const EMPTY_ROOT: &str = "000000";
    const ROOT: &str = "0000100130038679a597d998138965b1793503993691e50d3910347f2e6274327e0806c72d01311f5fec71d6df5709e7adb711835a80da5cca8ce27896a3e4a241977f62c142eb0132ef92a236c7622e6790bf8cfd13084488aa13d46109ecbbcca4579a4de8da2bdf01330ca9352121ed6a6ab793e124fb6453d96886cfdb9b8a399176ebc791313fd93001342161b614ecf10695066f82254149101292df15a8c7592ae3b44a2839247eeb1a01353604eb34e8720018f389a124eb7c05997b02c345a3f35ae06138584bb849d1b90136cfdf697dced7f0c07a09fdf2a7ba11641575205080cd9179f182a96ea01a69ca0137aea67da2ffc348458e4d8444477be2d6720da53c18414e0f6aed72ba4b5fd8b801383a6fc24a01fb88dc151dcc4d861e61e6951312c07be024d4df52ef5821dd77110139113ecaa04dfeeccef55576e7edc42a882cf9abb166509aca9598eb1b07b32f9a0161043cf8dd6f459a1edfdd1c7c25bd94263e0d00e650912967bb54ff899aa101bf01621cf41c547dba18db0b92b09a59d73c64e1be0f026ffcc7f56aad1f91938f07bd0163f4f3a285954a201a850e4331c7eeace387a83556dbac3c8ada406630b231110e0164297b13179a75cb0dd2288297d167c59cfd9fd29e20a0a20ad9a3c7e8f97ce308016596ee021457f86b14100eebe16efbe1c207f5914f59582cf7f64c986a3dca53040166ba6e9a80a103cac67f82938f6d6a63f1c87b9f4d1e06161ee63943461199b46e";
    const BRANCH_1: &str = "0003303032042530373864613866623936613539623364336535626632346131376334636135363130386334702543c7372bc57ca277115b4af975710ff9c0eb3d5f79f7cca53fb1d4e92e092539626637663639373131653835663135663166366230303362363438616234623061393837b18779913cb391d619ca4752d7119a075a5a00d1d14a05ffe65a0dd5c82e6f1a2563643437663164623461623133616532366262616637343939363037653563343237623039c510c8b8ca790c0a1769a0846db3da72a5c5e93d52c9ff81e6bbd1ed243bc4bd2564383935666333356461643266626332383834356163343862353962303231316264646634bb0a0e5467a7ae4ae4f67a7e531c5928e4edd095dad5b0a49dc0b8b645b1f116";
    const TERMINAL_BASIC: &str = "ff2830303130333137646334316666333130333830373566313432626261373237336133366539653630000000000023314350";
    const TERMINAL_VESTING: &str = "ff28383837613861303933643062346264383632623734316563323662613862663535373236313831360100000022ecb25c00ddf7adb0d0a547af45fd75f5baaa015d20a1bf59000000010001fa400000001176592e0000000022ecb25c00";
    const TERMINAL_HTLC: &str = "ff2830326436663565356138643834623462353232616133343737653437353464336635353836616265020000000000000001882b67dded32f197ef32ac605e19b163d02f796a882b67dded32f197ef32ac605e19b163d02f796a03ee8cda12a177778c0dbbb81af62edcd7fb3cf1f50f06653c8d92dcc22130b88801000231460000000000000001";

    #[test]
    fn it_can_calculate_hash() {
        let mut node = AccountsTreeNode::deserialize_from_vec(&hex::decode(EMPTY_ROOT).unwrap()).unwrap();
        assert_eq!(node.hash::<Blake2bHash>(), "ab29e6dc16755d0071eba349ebda225d15e4f910cb474549c47e95cb85ecc4d6".into());

        node = AccountsTreeNode::deserialize_from_vec(&hex::decode(ROOT).unwrap()).unwrap();
        assert_eq!(node.hash::<Blake2bHash>(), "4ff16759f1f4cc7274ff46864c9adef278c1a72a9ded784b882e9cbf313f4e7c".into());

        node = AccountsTreeNode::deserialize_from_vec(&hex::decode(BRANCH_1).unwrap()).unwrap();
        assert_eq!(node.hash::<Blake2bHash>(), "e208dcfb22e9130280c633ae753f4a1eb0a6caf71f1a8fce837a7d430b846f1f".into());

        node = AccountsTreeNode::deserialize_from_vec(&hex::decode(TERMINAL_BASIC).unwrap()).unwrap();
        assert_eq!(node.hash::<Blake2bHash>(), "3b1fcdc81825649f84a8dbf529b2677b4e5e866da96ee874c0b82f68b1afa7a3".into());

        node = AccountsTreeNode::deserialize_from_vec(&hex::decode(TERMINAL_VESTING).unwrap()).unwrap();
        assert_eq!(node.hash::<Blake2bHash>(), "aa73e73d559902bdd523b7a79962f3681d7431365999c49718fe5ac591c9daa7".into());

        node = AccountsTreeNode::deserialize_from_vec(&hex::decode(TERMINAL_HTLC).unwrap()).unwrap();
        assert_eq!(node.hash::<Blake2bHash>(), "7b27f721750a0413df093e278000018f1293dd46eb246a21c053fa91e27ccaa5".into());
    }
}
