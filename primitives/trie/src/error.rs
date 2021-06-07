use thiserror::Error;

/// An enum containing possible errors that can happen in the Merkle Radix Trie.
#[derive(Error, Debug, Clone, PartialEq)]
pub enum MerkleRadixTrieError {
    #[error("Prefix doesn't match node's key.")]
    WrongPrefix,
    #[error("Tried to query the child of a leaf node. Leaf nodes don't have children.")]
    LeavesHaveNoChildren,
    #[error("Tried to query the value of a branch node. Branch nodes don't have a value.")]
    BranchesHaveNoValue,
    #[error("Tried to query a child that does not exist.")]
    ChildDoesNotExist,
}
