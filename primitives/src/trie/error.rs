use std::fmt;

use thiserror::Error;

/// An enum containing possible errors that can happen in the Merkle Radix Trie.
#[derive(Clone, Error, Debug, Eq, PartialEq)]
pub enum MerkleRadixTrieError {
    #[error("Prefix doesn't match node's key.")]
    WrongPrefix,
    #[error("Tried to query the value of a branch node. Branch nodes don't have a value.")]
    BranchesHaveNoValue,
    #[error("Tried to query a child that does not exist.")]
    ChildDoesNotExist,
    #[error("Child is incomplete.")]
    ChildIsStump,
    #[error("Tried to store a value at the root node.")]
    RootCantHaveValue,
    #[error("Tree is already complete.")]
    TrieAlreadyComplete,
    #[error("Chunk does not match tree state.")]
    NonMatchingChunk,
    #[error("Root hash does not match expected hash after applying chunk.")]
    ChunkHashMismatch,
    #[error("Chunk is invalid: {0}")]
    InvalidChunk(&'static str),
    #[error("Trie is not complete")]
    IncompleteTrie,
    #[error("Serialization error")]
    Serialization(#[from] nimiq_serde::DeserializeError),
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct IncompleteTrie;

impl fmt::Display for IncompleteTrie {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        "the operation affects the incomplete part of the trie".fmt(f)
    }
}

impl std::error::Error for IncompleteTrie {}
