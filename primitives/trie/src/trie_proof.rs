use beserial::{Deserialize, Serialize};
use nimiq_hash::{Blake2bHash, Hash};

use crate::prefix_nibbles::PrefixNibbles;
use crate::trie_node::TrieNode;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrieProof<A: Serialize + Deserialize + Clone> {
    #[beserial(len_type(u16))]
    nodes: Vec<TrieNode<A>>,
    #[beserial(skip)]
    verified: bool,
}

impl<A: Serialize + Deserialize + Clone> TrieProof<A> {
    pub fn new(nodes: Vec<TrieNode<A>>) -> TrieProof<A> {
        TrieProof {
            nodes,
            verified: false,
        }
    }

    pub fn verify(&mut self) -> bool {
        let mut children: Vec<TrieNode<A>> = Vec::new();

        for node in &self.nodes {
            // If node is a branch node, validate its children.
            if node.is_branch() {
                while let Some(child) = children.pop() {
                    if node.prefix().is_prefix_of(child.prefix()) {
                        let hash = child.hash::<Blake2bHash>();

                        // If the child is not valid, return false.
                        if node.get_child_hash(child.prefix()).unwrap() != &hash
                            || &node.get_child_prefix(child.prefix()).unwrap() != child.prefix()
                        {
                            return false;
                        }
                    } else {
                        children.push(child);
                        break;
                    }
                }
            }
            children.push(node.clone());
        }

        let root_nibbles: PrefixNibbles = "".parse().unwrap();

        let valid =
            children.len() == 1 && children[0].prefix() == &root_nibbles && children[0].is_branch();

        self.verified = valid;

        valid
    }

    pub fn get_value(&self, key: &PrefixNibbles) -> Option<A> {
        assert!(
            self.verified,
            "AccountsProof must be verified before retrieving accounts. Call verify() first."
        );

        for node in &self.nodes {
            if let TrieNode::TerminalNode { prefix, value } = node {
                if prefix == key {
                    return Some(value.clone());
                }
            }
        }

        None
    }

    pub fn root_hash(&self) -> Blake2bHash {
        self.nodes.last().unwrap().hash()
    }

    pub fn nodes(&self) -> &Vec<TrieNode<A>> {
        &self.nodes
    }
}

#[cfg(test)]
mod tests {
    use crate::trie_node::TrieNodeChild;

    use super::*;

    #[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
    struct TestAccount {
        balance: u64,
    }

    #[test]
    fn it_can_verify() {
        /*
         * We're going to construct three proofs based on this tree:
         *
         *      R1
         *      |
         *      B1
         *    / |  \
         *   T1 B2 T2
         *     / \
         *    T3 T4
         */

        let nibbles1: PrefixNibbles = "0011111111111111111111111111111111111111".parse().unwrap();
        let value1 = TestAccount { balance: 25 };
        let t1 = TrieNode::new_terminal(nibbles1.clone(), value1.clone());

        let nibbles2: PrefixNibbles = "0033333333333333333333333333333333333333".parse().unwrap();
        let value2 = TestAccount { balance: 1 };
        let t2 = TrieNode::new_terminal(nibbles2.clone(), value2.clone());

        let nibbles3: PrefixNibbles = "0020000000000000000000000000000000000000".parse().unwrap();
        let value3 = TestAccount { balance: 1332 };
        let t3 = TrieNode::new_terminal(nibbles3.clone(), value3.clone());

        let nibbles4: PrefixNibbles = "0022222222222222222222222222222222222222".parse().unwrap();
        let value4 = TestAccount { balance: 93 };
        let t4 = TrieNode::new_terminal(nibbles4.clone(), value4.clone());

        let b2 = TrieNode::new_branch(
            "002".parse().unwrap(),
            [
                Some(TrieNodeChild {
                    suffix: "0000000000000000000000000000000000000".parse().unwrap(),
                    hash: t3.hash(),
                }),
                None,
                Some(TrieNodeChild {
                    suffix: "2222222222222222222222222222222222222".parse().unwrap(),
                    hash: t4.hash(),
                }),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            ],
        );

        let b1 = TrieNode::new_branch(
            "00".parse().unwrap(),
            [
                None,
                Some(TrieNodeChild {
                    suffix: "11111111111111111111111111111111111111".parse().unwrap(),
                    hash: t1.hash(),
                }),
                Some(TrieNodeChild {
                    suffix: "2".parse().unwrap(),
                    hash: b2.hash(),
                }),
                Some(TrieNodeChild {
                    suffix: "33333333333333333333333333333333333333".parse().unwrap(),
                    hash: t2.hash(),
                }),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            ],
        );

        let r1 = TrieNode::new_branch(
            "".parse().unwrap(),
            [
                Some(TrieNodeChild {
                    suffix: "00".parse().unwrap(),
                    hash: b1.hash(),
                }),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            ],
        );

        // The first proof proves the 4 terminal nodes (T1, T2, T3 and T4)
        let mut proof1 = TrieProof::new(vec![
            t1.clone(),
            t3.clone(),
            t4.clone(),
            b2.clone(),
            t2,
            b1.clone(),
            r1.clone(),
        ]);
        assert!(proof1.verify());
        assert_eq!(value1, proof1.get_value(&nibbles1).unwrap());
        assert_eq!(value2, proof1.get_value(&nibbles2).unwrap());
        assert_eq!(value3, proof1.get_value(&nibbles3).unwrap());
        assert_eq!(value4, proof1.get_value(&nibbles4).unwrap());

        // The second proof proves the 2 leftmost terminal nodes (T1 and T3)
        let mut proof2 = TrieProof::new(vec![t1, t3, b2.clone(), b1.clone(), r1.clone()]);
        assert!(proof2.verify());
        assert_eq!(value1, proof2.get_value(&nibbles1).unwrap());
        assert_eq!(value3, proof2.get_value(&nibbles3).unwrap());
        assert_eq!(None, proof2.get_value(&nibbles2));
        assert_eq!(None, proof2.get_value(&nibbles4));

        // The third proof just proves T4
        let mut proof3 = TrieProof::new(vec![t4, b2, b1, r1.clone()]);
        assert!(proof3.verify());
        assert_eq!(value4, proof3.get_value(&nibbles4).unwrap());
        assert_eq!(None, proof3.get_value(&nibbles1));
        assert_eq!(None, proof3.get_value(&nibbles2));
        assert_eq!(None, proof3.get_value(&nibbles3));

        // It must return the correct root hash
        assert!(proof1.root_hash() == r1.hash());
    }
}
