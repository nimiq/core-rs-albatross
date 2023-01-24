use std::ops::{Deref, DerefMut};

use beserial::{Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};

use crate::{key_nibbles::KeyNibbles, trie::trie_node::TrieNode};

/// A wrapper around the TrieNode to be used when we send nodes over the network.
/// As an optimization, the TrieNode does not serialize its key,
/// because we usually already have it when requesting it from the database.
/// This, however, is not the case when sending nodes over the network.
///
/// The NetworkTrieNode ensures the key is sent with the node
/// and provides conversion methods to/from a TrieNode.
#[derive(Clone, Debug)]
pub struct NetworkTrieNode {
    node: TrieNode,
}

impl Serialize for NetworkTrieNode {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = 0;
        size += self.node.key.serialize(writer)?;
        size += self.node.serialize(writer)?;
        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size = 0;
        size += self.node.key.serialized_size();
        size += self.node.serialized_size();
        size
    }
}

impl Deserialize for NetworkTrieNode {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let key: KeyNibbles = Deserialize::deserialize(reader)?;
        let mut node: TrieNode = Deserialize::deserialize(reader)?;
        node.key = key;
        Ok(NetworkTrieNode { node })
    }
}

impl From<TrieNode> for NetworkTrieNode {
    fn from(node: TrieNode) -> Self {
        Self { node }
    }
}

impl From<NetworkTrieNode> for TrieNode {
    fn from(node: NetworkTrieNode) -> Self {
        node.node
    }
}

impl Deref for NetworkTrieNode {
    type Target = TrieNode;

    fn deref(&self) -> &Self::Target {
        &self.node
    }
}

impl DerefMut for NetworkTrieNode {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.node
    }
}

impl AsRef<TrieNode> for NetworkTrieNode {
    fn as_ref(&self) -> &TrieNode {
        &self.node
    }
}

impl AsMut<TrieNode> for NetworkTrieNode {
    fn as_mut(&mut self) -> &mut TrieNode {
        &mut self.node
    }
}
