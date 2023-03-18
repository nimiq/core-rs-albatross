use beserial::Deserialize;
use nimiq_primitives::key_nibbles::KeyNibbles;

/// Read Operations that a Data Store needs to implement to interact with
/// contracts in the Accounts Trie.
pub trait DataStoreReadOps {
    fn get<T: Deserialize>(&self, key: &KeyNibbles) -> Option<T>;
}
