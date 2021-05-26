use beserial::{Deserialize, Serialize};
use nimiq_hash::Blake2bHash;

use crate::trie_node::TrieNode;
use crate::trie_proof::TrieProof;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrieChunk<A: Serialize + Deserialize + Clone> {
    #[beserial(len_type(u16))]
    pub nodes: Vec<TrieNode<A>>,
    pub proof: TrieProof<A>,
}

impl<A: Serialize + Deserialize + Clone> TrieChunk<A> {
    pub fn new(nodes: Vec<TrieNode<A>>, proof: TrieProof<A>) -> TrieChunk<A> {
        TrieChunk { nodes, proof }
    }

    pub fn verify(&mut self) -> bool {
        if !self.proof.verify() {
            return false;
        }

        let mut last_prefix = Option::None;

        for node in &self.nodes {
            if last_prefix > Option::Some(node.prefix()) {
                return false;
            }

            last_prefix = Option::Some(node.prefix());
        }

        if last_prefix > Option::Some(self.tail().prefix()) {
            return false;
        }

        true
    }

    pub fn head(&self) -> &TrieNode<A> {
        self.nodes.get(0).unwrap_or_else(|| self.tail())
    }

    pub fn terminal_nodes(&self) -> Vec<&TrieNode<A>> {
        let mut vec = Vec::with_capacity(self.nodes.len() + 1);

        for node in &self.nodes {
            vec.push(node)
        }

        vec.push(self.tail());

        vec
    }

    pub fn tail(&self) -> &TrieNode<A> {
        self.proof.nodes().get(0).unwrap()
    }

    pub fn root(&self) -> Blake2bHash {
        self.proof.root_hash()
    }

    pub fn last_terminal_string(&self) -> Option<String> {
        Some(self.tail().prefix().to_string())
    }
}
