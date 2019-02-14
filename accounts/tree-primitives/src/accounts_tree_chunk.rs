use beserial::{Deserialize, Serialize};
use hash::Blake2bHash;

use crate::accounts_proof::AccountsProof;
use crate::accounts_tree_node::AccountsTreeNode;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AccountsTreeChunk {
    #[beserial(len_type(u16))]
    pub nodes: Vec<AccountsTreeNode>,
    pub proof: AccountsProof,
}

impl AccountsTreeChunk {
    pub fn new(nodes: Vec<AccountsTreeNode>, proof: AccountsProof) -> AccountsTreeChunk {
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
        return true;
    }

    pub fn len(&self) -> usize { self.nodes.len() + 1 }

    #[inline]
    pub fn head(&self) -> &AccountsTreeNode { self.nodes.get(0).unwrap_or(self.tail()) }

    #[inline]
    pub fn terminal_nodes(&self) -> Vec<&AccountsTreeNode> {
        let mut vec = Vec::with_capacity(self.len());
        for node in &self.nodes {
            vec.push(node)
        }
        vec.push(self.tail());
        return vec;
    }

    #[inline]
    pub fn tail(&self) -> &AccountsTreeNode { self.proof.nodes().get(0).unwrap() }

    pub fn root(&self) -> Blake2bHash { self.proof.root_hash() }

    pub fn last_terminal_string(&self) -> Option<String> {
        Some(self.tail().prefix().to_string())
    }
}
