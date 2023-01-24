use log::error;

use beserial::{Deserialize, Serialize};
use nimiq_hash::Blake2bHash;

use crate::key_nibbles::KeyNibbles;
use crate::trie::{network_trie_node::NetworkTrieNode, trie_node::TrieNode};

/// A Merkle proof of the inclusion of some leaf nodes in the Merkle Radix Trie. The
/// proof consists of the path from the leaves that we want to prove inclusion all the way up
/// to the root. For example, for the following trie:
///              R
///              |
///              B1
///          /   |   \
///        B2   L3   B3
///       / \        / \
///      L1 L2      L4 L5
/// If we want a proof for the nodes L1 and L3, the proof will consist of the nodes L1, B2, L3,
/// B1 and R. Note that:
///     1. Unlike Merkle proofs we don't need the adjacent branch nodes. That's because our
///        branch nodes already include the hashes of its children.
///     2. The nodes are always returned in post-order.
/// If any of the given keys doesn't exist this function just returns None.
/// The exclusion (non-inclusion) of keys in the Merkle Radix Trie could also be proven, but it
/// requires some light refactoring to the way proofs work.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TrieProof {
    #[beserial(len_type(u16))]
    // TODO: for hybrid nodes, this contains data. is this bad?
    pub nodes: Vec<NetworkTrieNode>,
}

impl TrieProof {
    pub fn new(nodes: Vec<TrieNode>) -> TrieProof {
        TrieProof {
            nodes: nodes.into_iter().map(NetworkTrieNode::from).collect(),
        }
    }

    /// Returns all of the leaf/hybrid nodes in the proof. These are the nodes that we are proving
    /// inclusion in the trie.
    pub fn values(&self) -> Vec<&TrieNode> {
        let mut nodes = Vec::new();

        for node in &self.nodes {
            if node.value.is_some() {
                nodes.push(node.as_ref());
            }
        }

        nodes
    }

    /// Verifies a proof against the given root hash. Note that this doesn't check that whatever keys
    /// we want to prove are actually included in the proof. For that we need to call leaf_nodes()
    /// and compare their keys to the ones we want.
    /// This function just checks that the proof is in fact a valid sub-trie and that its root
    /// matches the given root hash.
    pub fn verify(&self, root_hash: &Blake2bHash) -> bool {
        // There must be nodes in the proof.
        if self.nodes.is_empty() {
            error!("There aren't any nodes in the trie proof!");
            return false;
        }

        // We'll use this vector to temporarily store child nodes before they are verified.
        let mut children: Vec<&TrieNode> = Vec::new();

        // Check that the proof is a valid trie.
        for node in &self.nodes {
            // If the node has children, validate them.
            if node.has_children() {
                // Pop the last node from the children.
                while let Some(child) = children.pop() {
                    // If the node is a prefix of the child node, we need to verify that it is a
                    // correct child node.
                    if node.key.is_prefix_of(&child.key) {
                        // Get the hash and key of the child from the parent node.
                        let (child_hash, child_key) =
                            match (node.child(&child.key), node.child_key(&child.key, &None)) {
                                (Ok(c), Ok(k)) => (&c.hash, k),
                                _ => return false,
                            };

                        // The child node must match the hash and the key, otherwise the proof is
                        // invalid.
                        if child_hash != &child.hash_assert::<Blake2bHash>()
                            || child_key != child.key
                        {
                            error!("The child node doesn't match the given hash and/or key. Got hash {}, child has hash {}. Got key {}, child has key {}.",
                                   child_hash, child.hash_assert::<Blake2bHash>(), child_key, child.key);
                            return false;
                        }
                    }
                    // If the node is not a prefix of the child node, then we put the child node
                    // back into the children and exit the loop.
                    else {
                        children.push(child);
                        break;
                    }
                }
            }

            // Put the current node into the children and move to the next node in the proof.
            children.push(node);
        }

        if children.len() != 1 {
            error!("There must be only one child now, the root node. Otherwise there are unverified nodes and the proof is invalid. There are {} remaining nodes.", children.len());
            return false;
        }

        let root = children.pop().unwrap();

        if root.key != KeyNibbles::ROOT {
            error!(
                "The root node doesn't have the correct key! It has key {}.",
                root.key,
            );
            return false;
        }

        // And must match the hash given as the root hash.
        if &root.hash_assert::<Blake2bHash>() != root_hash {
            error!(
                "The root node doesn't have the correct hash! It has hash {}, but it should be {}.",
                root.hash_assert::<Blake2bHash>(),
                root_hash
            );
            return false;
        }

        // The proof is valid!
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nimiq_hash::Hash;
    use nimiq_test_log::test;

    // We're going to construct proofs based on this tree:
    //
    //        R
    //        |
    //        B1
    //     /  |   \
    //    L1  B2  L2
    //       / \
    //      L3 L4
    //
    #[test]
    fn verify_works() {
        let key_l1: KeyNibbles = "0011".parse().unwrap();
        let l1 = TrieNode::new_leaf(key_l1.clone(), vec![1]);

        let key_l2: KeyNibbles = "0033".parse().unwrap();
        let l2 = TrieNode::new_leaf(key_l2.clone(), vec![2]);

        let key_l3: KeyNibbles = "0020".parse().unwrap();
        let l3 = TrieNode::new_leaf(key_l3.clone(), vec![3]);

        let key_l4: KeyNibbles = "0022".parse().unwrap();
        let l4 = TrieNode::new_leaf(key_l4.clone(), vec![4]);

        let key_b2: KeyNibbles = "002".parse().unwrap();
        let mut b2 = TrieNode::new_empty(key_b2.clone());
        b2.put_child(&key_l3, l3.hash_assert()).unwrap();
        b2.put_child(&key_l4, l4.hash_assert()).unwrap();

        let key_b1: KeyNibbles = "00".parse().unwrap();
        let mut b1 = TrieNode::new_empty(key_b1.clone());
        b1.put_child(&key_l1, l1.hash_assert()).unwrap();
        b1.put_child(&key_b2, b2.hash_assert()).unwrap();
        b1.put_child(&key_l2, l2.hash_assert()).unwrap();

        let key_r: KeyNibbles = "".parse().unwrap();
        let mut r = TrieNode::new_empty(key_r);
        r.put_child(&key_b1, b1.hash_assert()).unwrap();

        let root_hash = r.hash_assert();
        let wrong_root_hash = ":-E".hash::<Blake2bHash>();

        // Correct proofs.
        let proof1 = TrieProof::new(vec![
            l1.clone(),
            l3.clone(),
            l4.clone(),
            b2.clone(),
            l2.clone(),
            b1.clone(),
            r.clone(),
        ]);
        assert!(proof1.verify(&root_hash));
        assert!(!proof1.verify(&wrong_root_hash));

        let proof2 = TrieProof::new(vec![
            l1.clone(),
            l3.clone(),
            b2.clone(),
            b1.clone(),
            r.clone(),
        ]);
        assert!(proof2.verify(&root_hash));
        assert!(!proof2.verify(&wrong_root_hash));

        let proof3 = TrieProof::new(vec![l4.clone(), b2.clone(), b1.clone(), r.clone()]);
        assert!(proof3.verify(&root_hash));
        assert!(!proof3.verify(&wrong_root_hash));

        // Wrong proofs. Nodes in wrong order.
        let proof1 = TrieProof::new(vec![
            l1.clone(),
            b2.clone(),
            l3.clone(),
            l4.clone(),
            l2.clone(),
            b1.clone(),
            r.clone(),
        ]);
        assert!(!proof1.verify(&root_hash));
        assert!(!proof1.verify(&wrong_root_hash));

        let proof2 = TrieProof::new(vec![
            l1.clone(),
            l3.clone(),
            r.clone(),
            b2.clone(),
            b1.clone(),
        ]);
        assert!(!proof2.verify(&root_hash));
        assert!(!proof2.verify(&wrong_root_hash));

        let proof3 = TrieProof::new(vec![l4.clone(), b1.clone(), b2.clone(), r.clone()]);
        assert!(!proof3.verify(&root_hash));
        assert!(!proof3.verify(&wrong_root_hash));

        // Wrong proofs. Nodes with wrong hash.
        let mut b2_wrong = TrieNode::new_empty(key_b2.clone());
        b2_wrong.put_child(&key_l3, ":-[".hash()).unwrap();
        b2_wrong.put_child(&key_l4, l4.hash_assert()).unwrap();

        let mut b1_wrong = TrieNode::new_empty(key_b1.clone());
        b1_wrong.put_child(&key_l1, l1.hash_assert()).unwrap();
        b1_wrong.put_child(&key_b2, ":-[".hash()).unwrap();
        b1_wrong.put_child(&key_l2, l2.hash_assert()).unwrap();

        let proof1 = TrieProof::new(vec![
            l1.clone(),
            l3.clone(),
            l4.clone(),
            b2_wrong.clone(),
            l2.clone(),
            b1_wrong.clone(),
            r.clone(),
        ]);
        assert!(!proof1.verify(&root_hash));
        assert!(!proof1.verify(&wrong_root_hash));

        let proof2 = TrieProof::new(vec![
            l1.clone(),
            l3.clone(),
            b2_wrong,
            b1.clone(),
            r.clone(),
        ]);
        assert!(!proof2.verify(&root_hash));
        assert!(!proof2.verify(&wrong_root_hash));

        let proof3 = TrieProof::new(vec![l4.clone(), b2.clone(), b1_wrong, r.clone()]);
        assert!(!proof3.verify(&root_hash));
        assert!(!proof3.verify(&wrong_root_hash));

        // Wrong proofs. Nodes with wrong key.
        let key_l3_wrong: KeyNibbles = "00201".parse().unwrap();
        let mut b2_wrong = TrieNode::new_empty(key_b2);
        b2_wrong.put_child(&key_l3_wrong, l3.hash_assert()).unwrap();
        b2_wrong.put_child(&key_l4, l4.hash_assert()).unwrap();

        let key_b2_wrong: KeyNibbles = "003".parse().unwrap();
        let mut b1_wrong = TrieNode::new_empty(key_b1);
        b1_wrong.put_child(&key_l1, l1.hash_assert()).unwrap();
        b1_wrong.put_child(&key_b2_wrong, b2.hash_assert()).unwrap();
        b1_wrong.put_child(&key_l2, l2.hash_assert()).unwrap();

        let proof1 = TrieProof::new(vec![
            l1.clone(),
            l3.clone(),
            l4.clone(),
            b2_wrong.clone(),
            l2,
            b1_wrong.clone(),
            r.clone(),
        ]);
        assert!(!proof1.verify(&root_hash));
        assert!(!proof1.verify(&wrong_root_hash));

        let proof2 = TrieProof::new(vec![l1, l3, b2_wrong, b1, r.clone()]);
        assert!(!proof2.verify(&root_hash));
        assert!(!proof2.verify(&wrong_root_hash));

        let proof3 = TrieProof::new(vec![l4, b2, b1_wrong, r]);
        assert!(!proof3.verify(&root_hash));
        assert!(!proof3.verify(&wrong_root_hash));
    }
}
