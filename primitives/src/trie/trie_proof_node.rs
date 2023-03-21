use log::error;

use beserial::{Deserialize, Serialize, SerializeWithLength, WriteBytesExt};
use nimiq_hash::{Blake2bHash, Hash, SerializeContent};

use std::io;

use crate::{
    key_nibbles::KeyNibbles,
    trie::{
        error::MerkleRadixTrieError,
        trie_node::{Iter, TrieNode, TrieNodeChild},
    },
};

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
    pub children: [Option<TrieNodeChild>; 16],
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[repr(u8)]
enum ProofValue {
    None,
    LeafValue(#[beserial(len_type(u16))] Vec<u8>),
    HybridHash(Blake2bHash),
    HybridValue(#[beserial(len_type(u16))] Vec<u8>),
}

impl From<TrieNode> for TrieProofNode {
    fn from(node: TrieNode) -> Self {
        let value = match (node.has_children(), node.value) {
            (_, None) => ProofValue::None,
            (false, Some(val)) => ProofValue::LeafValue(val),
            (true, Some(val)) => ProofValue::HybridHash(val.hash()),
        };
        TrieProofNode {
            key: node.key,
            value,
            children: node.children,
        }
    }
}

impl TrieProofNode {
    pub fn include_value(node: TrieNode) -> Self {
        let value = match (node.has_children(), node.value) {
            (_, None) => panic!("can't include value of a node without one"),
            (false, Some(val)) => ProofValue::LeafValue(val),
            (true, Some(val)) => ProofValue::HybridValue(val),
        };
        TrieProofNode {
            key: node.key,
            value,
            children: node.children,
        }
    }
}

impl TrieProofNode {
    pub fn into_value(self) -> Option<Vec<u8>> {
        match self.value {
            ProofValue::None => None,
            ProofValue::LeafValue(value) => Some(value),
            ProofValue::HybridHash(_) => None,
            ProofValue::HybridValue(value) => Some(value),
        }
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

    pub fn value(&self) -> Option<&Vec<u8>> {
        if let ProofValue::Value(value) = &self.value {
            return Some(value);
        }
        None
    }
}

impl SerializeContent for TrieProofNode {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> {
        let mut size = 0;
        size += self.key.serialize(writer).unwrap();
        size += 1;
        match &self.value {
            ProofValue::None => {
                writer.write_u8(0).unwrap();
            }
            ProofValue::LeafValue(val) => {
                writer.write_u8(1).unwrap();
                size += val.serialize::<u16, _>(writer).unwrap();
            }
            ProofValue::HybridHash(val_hash) => {
                writer.write_u8(2).unwrap();
                size += val_hash.serialize(writer).unwrap();
            }
            ProofValue::HybridValue(val) => {
                writer.write_u8(2).unwrap();
                size += val.hash::<Blake2bHash>().serialize(writer).unwrap();
            }
        }
        size += self.children.serialize(writer).unwrap();
        Ok(size)
    }
}

impl Hash for TrieProofNode {}

#[cfg(test)]
mod test {
    use crate::key_nibbles::KeyNibbles;
    use crate::trie::trie_node::TrieNode;
    use crate::trie::trie_proof_node::TrieProofNode;
    use nimiq_hash::{Blake2bHash, Hash};

    #[test]
    #[should_panic = "can't include value of a node without one"]
    fn include_value_root() {
        TrieProofNode::include_value(TrieNode::new_root());
    }

    #[test]
    #[should_panic = "can't include value of a node without one"]
    fn include_value_branch() {
        let key: KeyNibbles = "cfb986".parse().unwrap();
        let child_key_1 = "cfb986f5a".parse().unwrap();
        let child_key_2 = "cfb986ab9".parse().unwrap();
        let child_key_3 = "cfb9860f6".parse().unwrap();
        let child_key_4 = "cfb986d50".parse().unwrap();

        let mut branch_node = TrieNode::new_empty(key);
        branch_node
            .put_child(&child_key_1, "child_1".hash())
            .unwrap();
        branch_node
            .put_child(&child_key_2, "child_2".hash())
            .unwrap();
        branch_node
            .put_child(&child_key_3, "child_3".hash())
            .unwrap();
        branch_node
            .put_child(&child_key_4, "child_4".hash())
            .unwrap();

        TrieProofNode::include_value(branch_node);
    }

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

        assert_eq!(
            root_node.hash_assert::<Blake2bHash>(),
            TrieProofNode::from(root_node).hash(),
        );
        assert_eq!(
            leaf_node.hash_assert::<Blake2bHash>(),
            TrieProofNode::from(leaf_node.clone()).hash(),
        );
        assert_eq!(
            leaf_node.hash_assert::<Blake2bHash>(),
            TrieProofNode::include_value(leaf_node).hash(),
        );
        assert_eq!(
            hybrid_node.hash_assert::<Blake2bHash>(),
            TrieProofNode::from(hybrid_node.clone()).hash(),
        );
        assert_eq!(
            hybrid_node.hash_assert::<Blake2bHash>(),
            TrieProofNode::include_value(hybrid_node).hash(),
        );
        assert_eq!(
            branch_node.hash_assert::<Blake2bHash>(),
            TrieProofNode::from(branch_node).hash(),
        );
    }
}
