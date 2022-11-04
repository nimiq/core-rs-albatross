use beserial::SerializingError;
use thiserror::Error;

/// An enum containing possible errors that can happen in the Merkle Radix Trie.
#[derive(Error, Debug, Eq, PartialEq)]
pub enum MerkleRadixTrieError {
    #[error("Prefix doesn't match node's key.")]
    WrongPrefix,
    #[error("Tried to query the value of a branch node. Branch nodes don't have a value.")]
    BranchesHaveNoValue,
    #[error("Tried to query a child that does not exist.")]
    ChildDoesNotExist,
    #[error("Tried to store a value at the root node.")]
    RootCantHaveValue,
    #[error("Failed to (de)serialize a value.")]
    SerializationFailed(SerializingError),
}

impl From<SerializingError> for MerkleRadixTrieError {
    fn from(err: SerializingError) -> Self {
        MerkleRadixTrieError::SerializationFailed(err)
    }
}
