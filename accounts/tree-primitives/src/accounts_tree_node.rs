use std::io;
use std::iter;
use std::slice;

use account::AccountsTreeLeave;
use beserial::{Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};
use hash::{Blake2bHash, Hash, SerializeContent};

use crate::address_nibbles::AddressNibbles;

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Debug, Serialize, Deserialize)]
pub struct AccountsTreeNodeChild {
    pub suffix: AddressNibbles,
    pub hash: Blake2bHash,
}

pub const NO_CHILDREN: [Option<AccountsTreeNodeChild>; 16] = [
    None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None,
];

#[derive(PartialEq, Eq, PartialOrd, Ord, Copy, Clone, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum AccountsTreeNodeType {
    BranchNode = 0x00,
    TerminalNode = 0xff,
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Debug)]
pub enum AccountsTreeNode<A: AccountsTreeLeave> {
    BranchNode {
        prefix: AddressNibbles,
        children: Box<[Option<AccountsTreeNodeChild>; 16]>,
    },
    TerminalNode {
        prefix: AddressNibbles,
        account: A,
    },
}

impl<A: AccountsTreeLeave> AccountsTreeNode<A> {
    pub fn new_terminal(prefix: AddressNibbles, account: A) -> Self {
        AccountsTreeNode::TerminalNode { prefix, account }
    }

    pub fn new_branch(
        prefix: AddressNibbles,
        children: [Option<AccountsTreeNodeChild>; 16],
    ) -> Self {
        AccountsTreeNode::BranchNode {
            prefix,
            children: Box::new(children),
        }
    }

    #[inline]
    pub fn is_terminal(&self) -> bool {
        match *self {
            AccountsTreeNode::TerminalNode { .. } => true,
            AccountsTreeNode::BranchNode { .. } => false,
        }
    }

    #[inline]
    pub fn is_branch(&self) -> bool {
        match *self {
            AccountsTreeNode::TerminalNode { .. } => false,
            AccountsTreeNode::BranchNode { .. } => true,
        }
    }

    pub fn get_child_hash(&self, prefix: &AddressNibbles) -> Option<&Blake2bHash> {
        match *self {
            AccountsTreeNode::TerminalNode { .. } => None,
            AccountsTreeNode::BranchNode { ref children, .. } => {
                if let Some(ref child) = children[self.get_child_index(prefix)?] {
                    return Some(&child.hash);
                }
                None
            }
        }
    }

    pub fn get_child_prefix(&self, prefix: &AddressNibbles) -> Option<AddressNibbles> {
        match *self {
            AccountsTreeNode::TerminalNode { .. } => None,
            AccountsTreeNode::BranchNode { ref children, .. } => {
                if let Some(ref child) = children[self.get_child_index(prefix)?] {
                    return Some(self.prefix() + &child.suffix);
                }
                None
            }
        }
    }

    #[inline]
    pub fn prefix(&self) -> &AddressNibbles {
        match *self {
            AccountsTreeNode::TerminalNode { ref prefix, .. } => &prefix,
            AccountsTreeNode::BranchNode { ref prefix, .. } => &prefix,
        }
    }

    #[inline]
    fn node_type(&self) -> AccountsTreeNodeType {
        match *self {
            AccountsTreeNode::TerminalNode { .. } => AccountsTreeNodeType::TerminalNode,
            AccountsTreeNode::BranchNode { .. } => AccountsTreeNodeType::BranchNode,
        }
    }

    #[inline]
    pub fn get_child_index(&self, prefix: &AddressNibbles) -> Option<usize> {
        assert!(
            self.prefix().is_prefix_of(prefix),
            "prefix {} is not a child of the current node {}",
            prefix,
            self.prefix()
        );
        prefix.get(self.prefix().len())
    }

    pub fn with_child(mut self, prefix: &AddressNibbles, hash: Blake2bHash) -> Option<Self> {
        let child_index = self.get_child_index(&prefix)?;
        let suffix = prefix.suffix(self.prefix().len() as u8);
        match self {
            AccountsTreeNode::TerminalNode { .. } => {
                return None;
            }
            AccountsTreeNode::BranchNode {
                ref mut children, ..
            } => {
                let child = AccountsTreeNodeChild { suffix, hash };
                children[child_index] = Some(child);
            }
        };
        Some(self)
    }

    pub fn without_child(mut self, suffix: AddressNibbles) -> Option<Self> {
        let child_index = self.get_child_index(&suffix)?;
        match self {
            AccountsTreeNode::TerminalNode { .. } => {
                return None;
            }
            AccountsTreeNode::BranchNode {
                ref mut children, ..
            } => {
                children[child_index] = None;
            }
        };
        Some(self)
    }

    pub fn with_account(mut self, new_account: A) -> Option<Self> {
        match &mut self {
            AccountsTreeNode::TerminalNode {
                ref mut account, ..
            } => {
                *account = new_account;
            }
            AccountsTreeNode::BranchNode { .. } => {
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

impl<A: AccountsTreeLeave> Serialize for AccountsTreeNode<A> {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size: usize = 0;
        size += Serialize::serialize(&self.node_type(), writer)?;
        size += Serialize::serialize(self.prefix(), writer)?;

        match *self {
            AccountsTreeNode::TerminalNode { ref account, .. } => {
                size += Serialize::serialize(&account, writer)?;
            }
            AccountsTreeNode::BranchNode { ref children, .. } => {
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

        match *self {
            AccountsTreeNode::TerminalNode { ref account, .. } => {
                size += Serialize::serialized_size(&account);
            }
            AccountsTreeNode::BranchNode { ref children, .. } => {
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

impl<A: AccountsTreeLeave> Deserialize for AccountsTreeNode<A> {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let node_type: AccountsTreeNodeType = Deserialize::deserialize(reader)?;
        let prefix: AddressNibbles = Deserialize::deserialize(reader)?;

        match node_type {
            AccountsTreeNodeType::TerminalNode => {
                let account: A = Deserialize::deserialize(reader)?;
                Ok(AccountsTreeNode::new_terminal(prefix, account))
            }
            AccountsTreeNodeType::BranchNode => {
                let child_count: u8 = Deserialize::deserialize(reader)?;
                let mut children = NO_CHILDREN;
                for _ in 0..child_count {
                    let child: AccountsTreeNodeChild = Deserialize::deserialize(reader)?;
                    if let Some(i) = child.suffix.get(0) {
                        children[i] = Some(child);
                    } else {
                        return Err(io::Error::from(io::ErrorKind::InvalidData).into());
                    }
                }
                Ok(AccountsTreeNode::new_branch(prefix, children))
            }
        }
    }
}

impl<A: AccountsTreeLeave> SerializeContent for AccountsTreeNode<A> {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> {
        Ok(self.serialize(writer)?)
    }
}

// Different hash implementation than std
#[allow(clippy::derive_hash_xor_eq)] // TODO: Shouldn't be necessary
impl<A: AccountsTreeLeave> Hash for AccountsTreeNode<A> {}

#[allow(clippy::type_complexity)]
type AccountsTreeNodeIter<'a> = Option<
    iter::FilterMap<
        slice::Iter<'a, Option<AccountsTreeNodeChild>>,
        fn(&Option<AccountsTreeNodeChild>) -> Option<&AccountsTreeNodeChild>,
    >,
>;

pub struct Iter<'a> {
    it: AccountsTreeNodeIter<'a>,
}

impl<'a> iter::Iterator for Iter<'a> {
    type Item = &'a AccountsTreeNodeChild;

    fn next(&mut self) -> Option<&'a AccountsTreeNodeChild> {
        if let Some(ref mut it) = self.it {
            return it.next();
        }
        None
    }
}

impl<'a, A: AccountsTreeLeave> iter::IntoIterator for &'a AccountsTreeNode<A> {
    type Item = &'a AccountsTreeNodeChild;
    type IntoIter = Iter<'a>;

    fn into_iter(self) -> Iter<'a> {
        match self {
            AccountsTreeNode::TerminalNode { .. } => Iter { it: None },
            AccountsTreeNode::BranchNode { ref children, .. } => Iter {
                it: Some(children.iter().filter_map(Option::as_ref)),
            },
        }
    }
}

type AccountsTreeNodeChildFilterMap<'a> = iter::FilterMap<
    slice::IterMut<'a, Option<AccountsTreeNodeChild>>,
    fn(&mut Option<AccountsTreeNodeChild>) -> Option<&mut AccountsTreeNodeChild>,
>;

pub struct IterMut<'a> {
    it: Option<AccountsTreeNodeChildFilterMap<'a>>,
}

impl<'a> iter::Iterator for IterMut<'a> {
    type Item = &'a mut AccountsTreeNodeChild;

    fn next(&mut self) -> Option<&'a mut AccountsTreeNodeChild> {
        if let Some(ref mut it) = self.it {
            return it.next();
        }
        None
    }
}

impl<'a, A: AccountsTreeLeave> iter::IntoIterator for &'a mut AccountsTreeNode<A> {
    type Item = &'a mut AccountsTreeNodeChild;
    type IntoIter = IterMut<'a>;

    fn into_iter(self) -> IterMut<'a> {
        match self {
            AccountsTreeNode::TerminalNode { .. } => IterMut { it: None },
            AccountsTreeNode::BranchNode {
                ref mut children, ..
            } => IterMut {
                it: Some(children.iter_mut().filter_map(Option::as_mut)),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use account::Account;

    use super::*;
    use nimiq_account::{BasicAccount, HashedTimeLockedContract, VestingContract};
    use nimiq_hash::HashOutput;
    use nimiq_keys::Address;
    use nimiq_primitives::coin::Coin;
    use nimiq_transaction::account::htlc_contract::{AnyHash, HashAlgorithm};

    const EMPTY_ROOT: &str = "000000";
    const ROOT: &str = "0000100130038679a597d998138965b1793503993691e50d3910347f2e6274327e0806c72d01311f5fec71d6df5709e7adb711835a80da5cca8ce27896a3e4a241977f62c142eb0132ef92a236c7622e6790bf8cfd13084488aa13d46109ecbbcca4579a4de8da2bdf01330ca9352121ed6a6ab793e124fb6453d96886cfdb9b8a399176ebc791313fd93001342161b614ecf10695066f82254149101292df15a8c7592ae3b44a2839247eeb1a01353604eb34e8720018f389a124eb7c05997b02c345a3f35ae06138584bb849d1b90136cfdf697dced7f0c07a09fdf2a7ba11641575205080cd9179f182a96ea01a69ca0137aea67da2ffc348458e4d8444477be2d6720da53c18414e0f6aed72ba4b5fd8b801383a6fc24a01fb88dc151dcc4d861e61e6951312c07be024d4df52ef5821dd77110139113ecaa04dfeeccef55576e7edc42a882cf9abb166509aca9598eb1b07b32f9a0161043cf8dd6f459a1edfdd1c7c25bd94263e0d00e650912967bb54ff899aa101bf01621cf41c547dba18db0b92b09a59d73c64e1be0f026ffcc7f56aad1f91938f07bd0163f4f3a285954a201a850e4331c7eeace387a83556dbac3c8ada406630b231110e0164297b13179a75cb0dd2288297d167c59cfd9fd29e20a0a20ad9a3c7e8f97ce308016596ee021457f86b14100eebe16efbe1c207f5914f59582cf7f64c986a3dca53040166ba6e9a80a103cac67f82938f6d6a63f1c87b9f4d1e06161ee63943461199b46e";
    const BRANCH_1: &str = "0003303032042530373864613866623936613539623364336535626632346131376334636135363130386334702543c7372bc57ca277115b4af975710ff9c0eb3d5f79f7cca53fb1d4e92e092539626637663639373131653835663135663166366230303362363438616234623061393837b18779913cb391d619ca4752d7119a075a5a00d1d14a05ffe65a0dd5c82e6f1a2563643437663164623461623133616532366262616637343939363037653563343237623039c510c8b8ca790c0a1769a0846db3da72a5c5e93d52c9ff81e6bbd1ed243bc4bd2564383935666333356461643266626332383834356163343862353962303231316264646634bb0a0e5467a7ae4ae4f67a7e531c5928e4edd095dad5b0a49dc0b8b645b1f116";
    const TERMINAL_BASIC: &str = "ff28666433346162373236356130653438633435346363626634633963363164666466363866396132320000000000000005dc";
    const TERMINAL_VESTING: &str = "ff28666433346162373236356130653438633435346363626634633963363164666466363866396132320100002fbf9bd9c800fd34ab7265a0e48c454ccbf4c9c61dfdf68f9a220000000000000001000000000003f480000002632e314a0000002fbf9bd9c800";
    const TERMINAL_HTLC: &str = "ff28666433346162373236356130653438633435346363626634633963363164666466363866396132320200000000000000001b215589344cf570d36bec770825eae30b73213924786862babbdb05e7c4430612135eb2a836812303daebe368963c60d22098a5e9f1ebcb8e54d0b7beca942a2a0a9d95391804fe8f0100000000000296350000000000000001";

    #[test]
    fn it_can_calculate_hash() {
        let mut node =
            AccountsTreeNode::<Account>::deserialize_from_vec(&hex::decode(EMPTY_ROOT).unwrap())
                .unwrap();
        assert_eq!(
            node.hash::<Blake2bHash>(),
            "ab29e6dc16755d0071eba349ebda225d15e4f910cb474549c47e95cb85ecc4d6".into()
        );

        node = AccountsTreeNode::deserialize_from_vec(&hex::decode(ROOT).unwrap()).unwrap();
        assert_eq!(
            node.hash::<Blake2bHash>(),
            "4ff16759f1f4cc7274ff46864c9adef278c1a72a9ded784b882e9cbf313f4e7c".into()
        );

        node = AccountsTreeNode::deserialize_from_vec(&hex::decode(BRANCH_1).unwrap()).unwrap();
        assert_eq!(
            node.hash::<Blake2bHash>(),
            "e208dcfb22e9130280c633ae753f4a1eb0a6caf71f1a8fce837a7d430b846f1f".into()
        );

        node =
            AccountsTreeNode::deserialize_from_vec(&hex::decode(TERMINAL_BASIC).unwrap()).unwrap();
        assert_eq!(
            node.hash::<Blake2bHash>(),
            "7c4b904074d07912698661ffcdfa8c0bf17ee1c3c0d6aae230e3b53ce7f23cf2".into()
        );

        node = AccountsTreeNode::deserialize_from_vec(&hex::decode(TERMINAL_VESTING).unwrap())
            .unwrap();
        assert_eq!(
            node.hash::<Blake2bHash>(),
            "45a043a02ff6e805734bee866bce8013c3272dcda4dbe063af142a6407896288".into()
        );

        node =
            AccountsTreeNode::deserialize_from_vec(&hex::decode(TERMINAL_HTLC).unwrap()).unwrap();
        assert_eq!(
            node.hash::<Blake2bHash>(),
            "b297ef941fb43fbd5d2afde680fb54a3e048b99343c471d404f81736eb1ea9d7".into()
        );
    }

    // This function is used to create the terminal node constants above. If you need to
    // generate new ones just uncomment out the test flag.
    //#[test]
    #[allow(dead_code)]
    fn create_terminal_constants() {
        let basic = BasicAccount {
            balance: Coin::from_u64_unchecked(1500),
        };

        let account = Account::Basic(basic);

        terminal_node_printer(account, "Basic");

        let vesting = VestingContract {
            balance: Coin::from_u64_unchecked(52500000000000),
            owner: "fd34ab7265a0e48c454ccbf4c9c61dfdf68f9a22".parse().unwrap(),
            start_time: 1,
            time_step: 259200,
            step_amount: Coin::from_u64_unchecked(2625000000000),
            total_amount: Coin::from_u64_unchecked(52500000000000),
        };

        let account = Account::Vesting(vesting);

        terminal_node_printer(account, "Vesting");

        let htlc = HashedTimeLockedContract {
            balance: Coin::ZERO,
            sender: "1b215589344cf570d36bec770825eae30b732139".parse().unwrap(),
            recipient: "24786862babbdb05e7c4430612135eb2a8368123".parse().unwrap(),
            hash_algorithm: HashAlgorithm::Sha256,
            hash_root: AnyHash::from(
                "daebe368963c60d22098a5e9f1ebcb8e54d0b7beca942a2a0a9d95391804fe8f",
            ),
            hash_count: 1,
            timeout: 169525,
            total_amount: Coin::from_u64_unchecked(1),
        };

        let account = Account::HTLC(htlc);

        terminal_node_printer(account, "HTLC");

        assert!(false);
    }

    fn terminal_node_printer(account: Account, name: &str) {
        let nibbles = AddressNibbles::from(
            &"fd34ab7265a0e48c454ccbf4c9c61dfdf68f9a22"
                .parse::<Address>()
                .unwrap(),
        );

        let node = AccountsTreeNode::new_terminal(nibbles, account);

        let mut bytes: Vec<u8> = Vec::with_capacity(node.serialized_size());

        node.serialize(&mut bytes).unwrap();

        println!("Serialized {} node:\n{}", name, hex::encode(bytes));

        let hash = node.hash::<Blake2bHash>();

        println!("{} node hash:\n{}", name, hex::encode(hash.as_bytes()));
    }
}
