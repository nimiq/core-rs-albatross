use std::io;

use byteorder::WriteBytesExt;
use log::error;
use nimiq_hash::{Blake2bHash, Hash, SerializeContent};
use nimiq_serde::{Deserialize, Serialize};

use crate::{
    key_nibbles::KeyNibbles,
    trie::{
        error::MerkleRadixTrieError,
        trie_node::{Iter, TrieNode, TrieNodeChild},
    },
};

use super::trie_node::BRANCHING_FACTOR;

/// A hashable [`TrieNode`] with less information.
///
/// A `TrieProofNode` saves less information than a [`TrieNode`], only as much
/// as is necessary to verify trie proofs.
///
/// The information that it does provide should be the same as that from the
/// [`TrieNode`] it was generated from. That means in particular that it hashes
/// to the same value.
///
/// Values of [`TrieNode`]s are part of the input to their hash function. Only
/// if they also have children, i.e. if they're hybrid nodes, the values are
/// not taken directly, but hashed first. This way, only the hashes of the
/// values of hybrid nodes have to be known in order to verify proofs, saving
/// some space.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TrieProofNode {
    pub key: KeyNibbles,
    value: ProofValue,
    pub children: [Option<TrieNodeChild>; BRANCHING_FACTOR],
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[repr(u8)]
enum ProofValue {
    None,
    LeafValue(Vec<u8>),
    HybridHash(Blake2bHash),
    HybridValue(Vec<u8>),
}

impl From<TrieNode> for TrieProofNode {
    fn from(node: TrieNode) -> Self {
        TrieProofNode::new(false, node)
    }
}

impl TrieProofNode {
    pub fn new(include_hybrid_value: bool, node: TrieNode) -> Self {
        let value = match (node.has_children(), node.value, include_hybrid_value) {
            (_, None, _) => ProofValue::None,
            (false, Some(val), _) => ProofValue::LeafValue(val),
            (true, Some(val), false) => ProofValue::HybridHash(val.hash()),
            (true, Some(val), true) => ProofValue::HybridValue(val),
        };
        TrieProofNode {
            key: node.key,
            value,
            children: node.children,
        }
    }
}

pub struct NodeValueMissing;

impl TrieProofNode {
    pub fn into_value(self) -> Result<Option<Vec<u8>>, NodeValueMissing> {
        Ok(match self.value {
            ProofValue::None => None,
            ProofValue::LeafValue(value) => Some(value),
            ProofValue::HybridHash(_) => return Err(NodeValueMissing),
            ProofValue::HybridValue(value) => Some(value),
        })
    }
    pub fn has_children(&self) -> bool {
        self.children.iter().any(|c| c.is_some())
    }
    pub fn child_index(&self, child_prefix: &KeyNibbles) -> Result<usize, MerkleRadixTrieError> {
        if !self.key.is_prefix_of(child_prefix) {
            error!(
                "Child's prefix {} is not a prefix of the node with key {}!",
                child_prefix, self.key,
            );
            return Err(MerkleRadixTrieError::WrongPrefix);
        }

        // Key length has to be smaller or equal to the child prefix length, so this will only panic
        // when `child_prefix` has the same length as `self.key()`.
        // PITODO: return error instead of unwrapping
        Ok(child_prefix.get(self.key.len()).unwrap())
    }

    pub fn child(&self, child_prefix: &KeyNibbles) -> Result<&TrieNodeChild, MerkleRadixTrieError> {
        if let Some(child) = &self.children[self.child_index(child_prefix)?] {
            return Ok(child);
        }
        Err(MerkleRadixTrieError::ChildDoesNotExist)
    }

    pub fn child_key(&self, child_prefix: &KeyNibbles) -> Result<KeyNibbles, MerkleRadixTrieError> {
        self.child(child_prefix)?.key(&self.key, &None)
    }

    pub fn iter_children(&self) -> Iter {
        Iter::from_children(&self.children)
    }
}

impl SerializeContent for TrieProofNode {
    fn serialize_content<W: io::Write, H>(&self, writer: &mut W) -> io::Result<()> {
        self.key.serialize(writer).unwrap();
        match &self.value {
            ProofValue::None => {
                writer.write_u8(0).unwrap();
            }
            ProofValue::LeafValue(val) => {
                writer.write_u8(1).unwrap();
                val.serialize(writer).unwrap();
            }
            ProofValue::HybridHash(val_hash) => {
                writer.write_u8(2).unwrap();
                val_hash.serialize(writer).unwrap();
            }
            ProofValue::HybridValue(val) => {
                writer.write_u8(2).unwrap();
                val.hash::<Blake2bHash>().serialize(writer).unwrap();
            }
        }
        self.children.serialize(writer).unwrap();
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use nimiq_hash::{Blake2bHash, Hash};

    use crate::{
        key_nibbles::KeyNibbles,
        trie::{trie_node::TrieNode, trie_proof_node::TrieProofNode},
    };

    #[test]
    fn hash_works() {
        let key: KeyNibbles = "cfb986".parse().unwrap();
        let child_key_1 = "cfb986f5a".parse().unwrap();
        let child_key_2 = "cfb986ab9".parse().unwrap();
        let child_key_3 = "cfb9860f6".parse().unwrap();
        let child_key_4 = "cfb986d50".parse().unwrap();

        let root_node = TrieNode::new_root();
        let leaf_node = TrieNode::new_leaf(key.clone(), vec![66]);
        let mut hybrid_node = TrieNode::new_leaf(key.clone(), vec![67]);
        let mut branch_node = TrieNode::new_empty(key);

        for node in [&mut hybrid_node, &mut branch_node] {
            node.put_child(&child_key_1, "child_1".hash()).unwrap();
            node.put_child(&child_key_2, "child_2".hash()).unwrap();
            node.put_child(&child_key_3, "child_3".hash()).unwrap();
            node.put_child(&child_key_4, "child_4".hash()).unwrap();
        }

        for include_hybrid_value in [false, true] {
            assert_eq!(
                root_node.hash_assert::<Blake2bHash>(),
                TrieProofNode::new(include_hybrid_value, root_node.clone()).hash(),
            );
            assert_eq!(
                leaf_node.hash_assert::<Blake2bHash>(),
                TrieProofNode::new(include_hybrid_value, leaf_node.clone()).hash(),
            );
            assert_eq!(
                hybrid_node.hash_assert::<Blake2bHash>(),
                TrieProofNode::new(include_hybrid_value, hybrid_node.clone()).hash(),
            );
            assert_eq!(
                branch_node.hash_assert::<Blake2bHash>(),
                TrieProofNode::new(include_hybrid_value, branch_node.clone()).hash(),
            );
        }
    }
}
