use beserial::{Deserialize, Serialize};
use nimiq_hash::Blake2bHash;

use crate::accounts_proof::AccountsProof;
use crate::accounts_tree_node::AccountsTreeNode;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AccountsTreeChunk<A: Serialize + Deserialize + Clone> {
    #[beserial(len_type(u16))]
    pub nodes: Vec<AccountsTreeNode<A>>,
    pub proof: AccountsProof<A>,
}

impl<A: Serialize + Deserialize + Clone> AccountsTreeChunk<A> {
    pub fn new(nodes: Vec<AccountsTreeNode<A>>, proof: AccountsProof<A>) -> AccountsTreeChunk<A> {
        AccountsTreeChunk { nodes, proof }
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

    #[inline]
    pub fn len(&self) -> usize {
        self.nodes.len() + 1
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        false
    }

    #[inline]
    pub fn head(&self) -> &AccountsTreeNode<A> {
        self.nodes.get(0).unwrap_or_else(|| self.tail())
    }

    #[inline]
    pub fn terminal_nodes(&self) -> Vec<&AccountsTreeNode<A>> {
        let mut vec = Vec::with_capacity(self.len());
        for node in &self.nodes {
            vec.push(node)
        }
        vec.push(self.tail());
        vec
    }

    #[inline]
    pub fn tail(&self) -> &AccountsTreeNode<A> {
        self.proof.nodes().get(0).unwrap()
    }

    pub fn root(&self) -> Blake2bHash {
        self.proof.root_hash()
    }

    pub fn last_terminal_string(&self) -> Option<String> {
        Some(self.tail().prefix().to_string())
    }
}
