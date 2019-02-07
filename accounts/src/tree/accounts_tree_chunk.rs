use beserial::{Deserialize, Serialize};
use hash::Blake2bHash;

use crate::accounts_proof::AccountsProof;

use super::AccountsTreeNode;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AccountsTreeChunk {
    #[beserial(len_type(u16))]
    pub(crate) nodes: Vec<AccountsTreeNode>,
    pub(crate) proof: AccountsProof,
}

impl AccountsTreeChunk {
    pub(crate) fn new(nodes: Vec<AccountsTreeNode>, proof: AccountsProof) -> AccountsTreeChunk {
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
    pub(crate) fn head(&self) -> &AccountsTreeNode { self.nodes.get(0).unwrap_or(self.tail()) }

    #[inline]
    pub(crate) fn terminalNodes(&self) -> Vec<&AccountsTreeNode> {
        let mut vec = Vec::with_capacity(self.len());
        for node in &self.nodes {
            vec.push(node)
        }
        vec.push(self.tail());
        return vec;
    }

    #[inline]
    pub(crate) fn tail(&self) -> &AccountsTreeNode { self.proof.nodes.get(0).unwrap() }

    pub fn root(&self) -> Blake2bHash { self.proof.root_hash() }
}
