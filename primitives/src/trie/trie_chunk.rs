use std::fmt::{self, Display};

use nimiq_serde::{option_max_size, Deserialize, Serialize, SerializedMaxSize};

use crate::{key_nibbles::KeyNibbles, trie::trie_proof::TrieProof};

/// The positive outcomes when committing a chunk.  
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrieChunkPushResult {
    /// The chunk was successfully applied.
    Applied,
    /// Reflects the case when the start key does not match.
    Ignored,
}

/// A helper structure for holding a trie chunk and the corresponding start key.
#[derive(Debug, Clone)]
pub struct TrieChunkWithStart {
    pub chunk: TrieChunk,
    pub start_key: KeyNibbles,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrieItem {
    pub key: KeyNibbles,
    pub value: Vec<u8>,
}

impl TrieItem {
    pub fn new(key: KeyNibbles, value: Vec<u8>) -> Self {
        Self { key, value }
    }
}

/// Common data structure for holding chunk items and proof.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrieChunk {
    /// The end of the chunk. The end key is exclusive.
    /// When set to None it means that it is the last trie chunk.
    pub end_key: Option<KeyNibbles>,
    pub items: Vec<TrieItem>,
    pub proof: TrieProof,
}

impl SerializedMaxSize for TrieChunk {
    const MAX_SIZE: usize = 0 
    + /*Option end_key*/ option_max_size(KeyNibbles::MAX_SIZE)
    + /*items*/ 1000;// ITODO
}

impl Display for TrieChunk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "TrieChunk {{ end_key: {:?}, items: #{}, proof: .. }}",
            self.end_key,
            self.items.len()
        )
    }
}

impl TrieChunk {
    pub fn new(end_key: Option<KeyNibbles>, items: Vec<TrieItem>, proof: TrieProof) -> Self {
        Self {
            end_key,
            items,
            proof,
        }
    }
}
