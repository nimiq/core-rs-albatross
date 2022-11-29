use std::borrow::Borrow;
use std::cmp;
use std::error::Error;
use std::fmt;
use std::fmt::Display;
use std::mem;
use std::ops;
use std::ops::RangeFrom;

use beserial::DeserializeWithLength;
use beserial::SerializeWithLength;
use log::error;

use beserial::{Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};
use nimiq_database::{Database, Environment, Transaction, WriteTransaction};
use nimiq_hash::Blake2bHash;

use crate::error::MerkleRadixTrieError;
use crate::key_nibbles::KeyNibbles;
use crate::trie_node::{RootData, TrieNode, TrieNodeKind};
use crate::trie_proof::TrieProof;

/// A Merkle Radix Trie is a hybrid between a Merkle tree and a Radix trie. Like a Merkle tree each
/// node contains the hashes of all its children. That creates a tree that is resistant to
/// unauthorized modification and allows proofs of inclusion and exclusion. Like a Radix trie each
/// node position is determined by its key, and it's space optimized by having each "only child" node
/// merged with its parent.
/// We keep all values at the edges of the trie, at the leaf nodes. The branch nodes keep only
/// references to its children. In this respect it is different from the Patricia Merkle Trie used
/// on other chains.
/// It is generic over the values and makes use of Nimiq's database for storage.
///
/// PITODO: Review use of unwrap/expect in the trie's methods.
#[derive(Debug)]
pub struct MerkleRadixTrie {
    db: Database,
}

/// Common data structure for holding chunk items and proof.
#[derive(Debug, Clone)]
pub struct TrieChunk {
    pub keys_end: Option<KeyNibbles>,
    pub items: Vec<(KeyNibbles, Vec<u8>)>,
    pub proof: TrieProof,
}

impl TrieChunk {
    pub fn new(
        keys_end: Option<KeyNibbles>,
        items: Vec<(KeyNibbles, Vec<u8>)>,
        proof: TrieProof,
    ) -> Self {
        Self {
            keys_end,
            items,
            proof,
        }
    }
}

impl Serialize for TrieChunk {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = 0;
        size += self.keys_end.serialize(writer)?;

        size += Serialize::serialize(&(self.items.len() as u32), writer)?;
        for item in self.items.iter() {
            size += item.0.serialize(writer)?;
            size += SerializeWithLength::serialize::<u16, _>(&item.1, writer)?;
        }

        size += self.proof.serialize(writer)?;
        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size = 0;
        size += self.keys_end.serialized_size();

        size += Serialize::serialized_size(&(self.items.len() as u32));
        for item in self.items.iter() {
            size += item.0.serialized_size();
            size += SerializeWithLength::serialized_size::<u16>(&item.1);
        }

        size += self.proof.serialized_size();
        size
    }
}

impl Deserialize for TrieChunk {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let keys_end = Deserialize::deserialize(reader)?;

        let num_items: u32 = Deserialize::deserialize(reader)?;
        let mut items = Vec::with_capacity(num_items as usize);
        for _ in 0..num_items {
            let key = Deserialize::deserialize(reader)?;
            let data = DeserializeWithLength::deserialize::<u16, _>(reader)?;
            items.push((key, data));
        }

        let proof = Deserialize::deserialize(reader)?;

        Ok(TrieChunk {
            keys_end,
            items,
            proof,
        })
    }
}

/// Counts the number of updates performed.
#[derive(Default)]
struct CountUpdates {
    branches: i8,
    hybrids: i8,
    leaves: i8,
}

impl CountUpdates {
    fn from_update(prev: Option<TrieNodeKind>, cur: Option<TrieNodeKind>) -> CountUpdates {
        let mut result = CountUpdates::default();
        result.apply_update(prev, cur);
        result
    }
    fn apply_update(&mut self, prev: Option<TrieNodeKind>, cur: Option<TrieNodeKind>) {
        if let Some(counter) = self.counter_mut(prev) {
            *counter -= 1;
        }
        if let Some(counter) = self.counter_mut(cur) {
            *counter += 1;
        }
    }
    fn is_empty(&self) -> bool {
        self.branches == 0 && self.hybrids == 0 && self.leaves == 0
    }
    fn counter_mut(&mut self, kind: Option<TrieNodeKind>) -> Option<&mut i8> {
        match kind {
            None => None,
            Some(TrieNodeKind::Root) => None,
            Some(TrieNodeKind::Branch) => Some(&mut self.branches),
            Some(TrieNodeKind::Hybrid) => Some(&mut self.hybrids),
            Some(TrieNodeKind::Leaf) => Some(&mut self.leaves),
        }
    }
    fn update_root_data(self, root_data: &mut RootData) {
        let apply_difference = |data: &mut u64, diff| {
            if diff >= 0 {
                *data += diff as u64;
            } else {
                *data -= (-(diff as i64)) as u64;
            }
        };
        apply_difference(&mut root_data.num_branches, self.branches);
        apply_difference(&mut root_data.num_hybrids, self.hybrids);
        apply_difference(&mut root_data.num_leaves, self.leaves);
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct IncompleteTrie;

impl fmt::Display for IncompleteTrie {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        "the operation affects the incomplete part of the trie".fmt(f)
    }
}

impl Error for IncompleteTrie {}

impl MerkleRadixTrie {
    /// Start a new Merkle Radix Trie with the given Environment and the given name.
    pub fn new(env: Environment, name: &str) -> Self {
        Self::new_impl(env, name, false)
    }

    pub fn new_incomplete(env: Environment, name: &str) -> Self {
        Self::new_impl(env, name, true)
    }

    fn new_impl(env: Environment, name: &str, incomplete: bool) -> Self {
        let db = env.open_database(name.to_string());

        let tree = MerkleRadixTrie { db };

        let mut txn = WriteTransaction::new(&env);
        if tree.get_root(&txn).is_none() {
            let root = if incomplete {
                TrieNode::new_root_incomplete()
            } else {
                TrieNode::new_root()
            };
            tree.put_node(&mut txn, &root);
        }
        txn.commit();

        tree
    }

    /// Returns the root hash of the Merkle Radix Trie.
    pub fn root_hash(&self, txn: &Transaction) -> Blake2bHash {
        self.get_root(txn).unwrap().hash_assert_complete()
    }

    pub fn is_complete(&self, txn: &Transaction) -> bool {
        self.get_root(txn)
            .unwrap()
            .root_data
            .unwrap()
            .incomplete
            .is_none()
    }

    /// Returns the number of branch nodes in the Merkle Radix Trie.
    pub fn num_branches(&self, txn: &Transaction) -> u64 {
        self.get_root(txn).unwrap().root_data.unwrap().num_branches
    }

    /// Returns the number of hybrid nodes in the Merkle Radix Trie.
    pub fn num_hybrids(&self, txn: &Transaction) -> u64 {
        self.get_root(txn).unwrap().root_data.unwrap().num_hybrids
    }

    /// Returns the number of leaf nodes in the Merkle Radix Trie.
    pub fn num_leaves(&self, txn: &Transaction) -> u64 {
        self.get_root(txn).unwrap().root_data.unwrap().num_leaves
    }

    #[cfg(test)]
    fn count_nodes(&self, txn: &Transaction) -> (u64, u64, u64) {
        let mut num_branches = 0;
        let mut num_hybrids = 0;
        let mut num_leaves = 0;

        let root = self
            .get_root(txn)
            .expect("The Merkle Radix Trie didn't have a root node!");
        let missing_range = root
            .root_data
            .clone()
            .expect("root node needs root data")
            .incomplete;
        let mut stack = vec![root];

        while let Some(item) = stack.pop() {
            for child in &item {
                match child.key(&item.key, &missing_range) {
                    // If it's a stump, don't count it.
                    Err(_) => continue,
                    Ok(child_key) => {
                        stack.push(self.get_node(txn, &child_key)
                            .expect("Failed to find the child of a Merkle Radix Trie node. The database must be corrupt!"));
                    }
                }
            }
            match item.kind() {
                None => panic!("Empty nodes mustn't exist in the database"),
                Some(TrieNodeKind::Root) => {}
                Some(TrieNodeKind::Branch) => num_branches += 1,
                Some(TrieNodeKind::Hybrid) => num_hybrids += 1,
                Some(TrieNodeKind::Leaf) => num_leaves += 1,
            }
        }

        assert_eq!(
            (
                self.num_branches(txn),
                self.num_hybrids(txn),
                self.num_leaves(txn)
            ),
            (num_branches, num_hybrids, num_leaves)
        );

        (num_branches, num_hybrids, num_leaves)
    }

    fn get_node(&self, txn: &Transaction, key: &KeyNibbles) -> Option<TrieNode> {
        let mut node: TrieNode = txn.get(&self.db, key)?;
        node.key = key.clone();
        Some(node)
    }

    fn put_node(&self, txn: &mut WriteTransaction, node: &TrieNode) {
        txn.put_reserve(&self.db, &node.key, node);
    }

    /// Get the value at the given key. If there's no leaf or hybrid node at the given key then it
    /// returns None.
    pub fn get<T: Deserialize>(
        &self,
        txn: &Transaction,
        key: &KeyNibbles,
    ) -> Result<Option<T>, IncompleteTrie> {
        self.check_within_complete_part(txn, key)?;
        Ok(self
            .get_raw(txn, key)
            .map(|v| T::deserialize_from_vec(&v).unwrap()))
    }

    fn get_raw(&self, txn: &Transaction, key: &KeyNibbles) -> Option<Vec<u8>> {
        self.get_node(txn, key)?.value
    }

    /// Returns a chunk of the Merkle Radix Trie that starts at the key `start` (which might or not
    /// be a part of the trie, if it is then it will be part of the chunk) and contains at most
    /// `size` leaf nodes.
    // FIXME This panics if a node in range can't be deserialized to T
    pub fn get_chunk<T: Deserialize>(
        &self,
        txn: &Transaction,
        start: &KeyNibbles,
        size: usize,
    ) -> Vec<T> {
        let chunk = self.get_trie_chunk(txn, start, size);

        chunk
            .into_iter()
            .map(|node| T::deserialize_from_vec(&node.value.unwrap()).unwrap())
            .collect()
    }

    pub fn put<T: Serialize>(
        &self,
        txn: &mut WriteTransaction,
        key: &KeyNibbles,
        value: T,
    ) -> Result<(), IncompleteTrie> {
        let missing_range = self.check_within_complete_part(txn, key)?;
        self.put_raw(txn, key, value.serialize_to_vec(), &missing_range);
        Ok(())
    }

    /// Insert a value into the Merkle Radix Trie at the given key. If the key already exists then
    /// it will overwrite it. You can't use this function to check the existence of a given key.
    fn put_raw(
        &self,
        txn: &mut WriteTransaction,
        key: &KeyNibbles,
        value: Vec<u8>,
        missing_range: &Option<RangeFrom<KeyNibbles>>,
    ) {
        // Start by getting the root node.
        let mut cur_node = self
            .get_root(txn)
            .expect("Merkle Radix Trie must have a root node!");

        // And initialize the root path.
        let mut root_path: Vec<TrieNode> = vec![];
        let mut count_updates;

        // Go down the trie until you find where to put the new value.
        loop {
            // If the current node key is no longer a prefix for the given key then we need to
            // split the node.
            if !cur_node.key.is_prefix_of(key) {
                // Check if the new node is a sibling or the parent of cur_node.
                if key.is_prefix_of(&cur_node.key) {
                    // The new node is the parent of the current node. Thus it needs to be a hybrid node.
                    let mut new_node = TrieNode::new_leaf(key.clone(), value);
                    new_node
                        .put_child(&cur_node.key, Blake2bHash::default())
                        .unwrap();
                    self.put_node(txn, &new_node);

                    // Push the new node into the root path.
                    root_path.push(new_node);
                    count_updates = CountUpdates {
                        hybrids: 1,
                        ..Default::default()
                    };
                } else {
                    // The new node is a sibling of the current node. Thus it is a leaf node.
                    let new_node = TrieNode::new_leaf(key.clone(), value);
                    self.put_node(txn, &new_node);

                    // We insert a new branch node as the parent of both the current node and the
                    // new node.
                    let mut new_parent = TrieNode::new_empty(cur_node.key.common_prefix(key));
                    new_parent.put_child_no_hash(&cur_node.key).unwrap();
                    new_parent
                        .put_child(&new_node.key, new_node.hash_assert_complete())
                        .unwrap();
                    self.put_node(txn, &new_parent);

                    count_updates = CountUpdates {
                        branches: 1,
                        leaves: 1,
                        ..Default::default()
                    };
                    // Push the parent node into the root path.
                    root_path.push(new_parent);
                }
                break;
            }

            // If the current node key is equal to the given key, we have found an existing node
            // with the given key. Update the value.
            if cur_node.key == *key {
                // Update the node and store it.
                // TODO This unwrap() will fail when attempting to store a value at the root node
                let prev_kind = cur_node.kind();
                cur_node.put_value(value).unwrap();
                self.put_node(txn, &cur_node);

                count_updates = CountUpdates::from_update(prev_kind, cur_node.kind());
                // Push the node into the root path.
                root_path.push(cur_node);
                break;
            }

            // Try to find a child of the current node that matches our key.
            match cur_node.child_key(key, missing_range) {
                // If no matching child exists, add a new child to the current node.
                Err(_) => {
                    // Create and store the new node.
                    let new_node = TrieNode::new_leaf(key.clone(), value);
                    self.put_node(txn, &new_node);

                    // Update the parent node and store it.
                    let old_kind = cur_node.kind();
                    cur_node
                        .put_child(&new_node.key, new_node.hash_assert_complete())
                        .unwrap();
                    self.put_node(txn, &cur_node);

                    count_updates = CountUpdates {
                        leaves: 1,
                        ..Default::default()
                    };
                    count_updates.apply_update(old_kind, cur_node.kind());
                    // Push the parent node into the root path.
                    root_path.push(cur_node);
                    break;
                }
                // If there's a child, then we update the current node and the root path, and
                // continue down the trie.
                Ok(child_key) => {
                    root_path.push(cur_node);
                    cur_node = self.get_node(txn, &child_key).unwrap();
                }
            }
        }

        // Update the keys and hashes of the nodes that we modified.
        self.update_keys(txn, root_path, count_updates);
    }

    pub fn get_chunk_with_proof(
        &self,
        txn: &Transaction,
        keys: ops::RangeFrom<KeyNibbles>,
        limit: usize,
    ) -> TrieChunk {
        let mut chunk = self.get_trie_chunk(txn, &keys.start, limit + 1);
        let first_not_included = if chunk.len() == limit + 1 {
            chunk.pop().map(|n| n.key)
        } else {
            None
        };
        let chunk: Vec<_> = chunk
            .into_iter()
            .map(|n| (n.key, n.value.unwrap()))
            .collect();
        let proof = if let Some((last, _)) = chunk.last() {
            // Get the proof for the last included item if available.
            self.get_proof(txn, vec![last])
                .expect("trie proof failed for last item")
        } else if let Some(pred) = self.get_predecessor(txn, &keys.start) {
            // Else for the last item already included in the trie.
            self.get_proof(txn, vec![&pred])
                .expect("trie proof failed for predecessor")
        } else {
            // Else just for the root if there are no items in the trie.
            TrieProof::new(vec![self.get_root(txn).unwrap()])
        };
        // The end can contain one nibble beyond the shared prefix of the next item not included and
        // the last item still included in the chunk.
        let end = first_not_included.map(|key| {
            let max_len = proof.nodes.first().unwrap().key.common_prefix(&key).len() + 1;
            key.slice(0, cmp::min(key.len(), max_len))
        });
        TrieChunk {
            keys_end: end,
            items: chunk,
            proof,
        }
    }

    /// Removes empty stump children on the rightmost path in the tree.
    /// These stumps are stored in the existing nodes to mark the (yet) missing parts of the partial tree.
    fn clear_stumps(&self, txn: &mut WriteTransaction, keys: ops::RangeFrom<KeyNibbles>) {
        // Start by getting the root node.
        let mut cur_node = self
            .get_root(txn)
            .expect("Merkle Radix Trie must have a root node!");

        let missing_range = cur_node
            .root_data
            .clone()
            .expect("root node needs root data")
            .incomplete;

        // And initialize the root path.
        let mut root_path: Vec<TrieNode> = vec![];
        let mut count_updates = CountUpdates::default();

        loop {
            // If we're no longer a prefix of the limit, we're done.
            if !cur_node.key.is_prefix_of(&keys.start) {
                break;
            }
            // Assume we have a tree with the following nodes:
            // 11
            // 12
            // 121
            // 122
            // 123
            // 13
            //   R
            //   |
            //   1
            //  /|\
            // 1 2 3
            //  /|\
            // 1 2 3

            // clear from 12..: this means clearing 2, 3 from node 1 // this can't happen, tree is too detailed
            // clear from 122..: this means clearing 2, 3 from node 12 and 3 from node 1
            // clear from 121..: this means clearing 1, 2, 3 from node 12 and 3 from node 1
            {
                let cur_digit = keys.start.get(cur_node.key.len());
                let is_last_digit = keys.start.len() <= cur_node.key.len() + 1;
                let prune_from = cur_digit.unwrap_or(0) // can only be `None` for the root node
                    + if is_last_digit { 0 } else { 1 };
                assert!(cur_node.key.is_empty() || cur_digit.is_some());
                let prev_kind = cur_node.kind();
                for child in &mut cur_node.children[prune_from..16] {
                    assert!(child
                        .as_ref()
                        .map(|c| c.is_stump(&cur_node.key, &missing_range))
                        .unwrap_or(true));
                    *child = None;
                }

                // If we only have one child remaining, merge with the parent node.
                if cur_node.value.is_none()
                    && !root_path.is_empty()
                    && cur_node.iter_children().count() == 1
                {
                    count_updates.apply_update(prev_kind, None);
                    txn.remove(&self.db, &cur_node.key);

                    let only_child_key = cur_node
                        .iter_children()
                        .next()
                        .unwrap()
                        .key(&cur_node.key, &missing_range)
                        .expect("no stump");

                    cur_node = root_path.pop().unwrap();
                    cur_node.put_child_no_hash(&only_child_key).unwrap();
                } else {
                    count_updates.apply_update(prev_kind, cur_node.kind());
                    self.put_node(txn, &cur_node);
                }
            }

            if cur_node.key.len() == keys.start.len() {
                // If we got to our actual key, we're done.
                root_path.push(cur_node);
                break;
            }

            // Try to find a child of the current node that matches our key.
            let maybe_child_key = cur_node.child_key(&keys.start, &missing_range);
            root_path.push(cur_node);
            match maybe_child_key {
                // If no matching child exists, we're done.
                Err(_) => break,
                // If there's a child, then we clear its stumps and continue down the trie.
                Ok(child_key) => cur_node = self.get_node(txn, &child_key).unwrap(),
            }
        }
        self.update_keys(txn, root_path, count_updates);
    }

    /// Marks empty stump children on the rightmost path in the tree.
    /// These stumps are stored in the existing nodes to mark the (yet) missing parts of the partial tree.
    /// We can know which children are missing by looking at the trie proof of this path.
    fn mark_stumps(
        &self,
        txn: &mut WriteTransaction,
        keys: ops::RangeFrom<KeyNibbles>,
        last_item_proof: &[TrieNode],
    ) -> Result<(), MerkleRadixTrieError> {
        let mut last_item_proof = last_item_proof.iter().rev();
        let missing_range = Some(keys.clone());

        // Start by getting the root node.
        let mut cur_node = self
            .get_root(txn)
            .expect("Merkle Radix Trie must have a root node!");

        let mut cur_proof = last_item_proof
            .next()
            .ok_or(MerkleRadixTrieError::InvalidChunk(
                "Proof must have at least a root node",
            ))?;

        // And initialize the root path.
        let mut root_path: Vec<TrieNode> = vec![];
        let mut count_updates = CountUpdates::default();

        // Go down the trie until you find where to put the new value.
        loop {
            // If the current node key doesn't match the proof key, we need to split the current
            // node.
            if cur_node.key != cur_proof.key {
                if !cur_proof.key.is_prefix_of(&cur_node.key) {
                    return Err(MerkleRadixTrieError::InvalidChunk(
                        "proof doesn't contain all intermediary nodes",
                    ));
                }
                let mut new_node = TrieNode::new_empty(cur_proof.key.clone());
                new_node.put_child_no_hash(&cur_node.key).unwrap();
                cur_node = new_node;
                count_updates.branches += 1;
            }

            // Mark everything as stump that's in the stump range and is contained in the proof.
            let start_idx = keys
                .start
                .get(cur_node.key.len())
                .map(|x| x + usize::from(cur_node.key.len() + 1 < keys.start.len()))
                .unwrap_or(0);
            let prev_kind = cur_node.kind();
            for i in start_idx..16 {
                assert!(cur_node.children[i].is_none());
                if cur_proof.children[i].is_some() {
                    cur_node.children[i] = cur_proof.children[i].clone();
                }
            }
            count_updates.apply_update(prev_kind, cur_node.kind());

            self.put_node(txn, &cur_node);

            match last_item_proof.next() {
                Some(n) => cur_proof = n,
                None => {
                    root_path.push(cur_node);
                    break;
                }
            }

            // Try to find a child of the current node that matches our missing key range.
            let maybe_child_key = cur_node.child_key(&keys.start, &missing_range);
            root_path.push(cur_node);
            match maybe_child_key {
                // If no matching child exists, we're done.
                Err(_) => break,
                // If there's a child, we continue down the trie.
                Ok(child_key) => cur_node = self.get_node(txn, &child_key).unwrap(),
            }
        }

        // Update the keys and hashes of the nodes that we modified.
        self.update_keys(txn, root_path, count_updates);
        Ok(())
    }

    /// `keys_start` is inclusive, `keys_end` is exclusive.
    pub fn put_chunk(
        &self,
        txn: &mut WriteTransaction,
        keys_start: KeyNibbles,
        chunk: TrieChunk,
        expected_hash: Blake2bHash,
    ) -> Result<(), MerkleRadixTrieError> {
        match self.get_root(txn).unwrap().root_data.unwrap().incomplete {
            Some(i) if i.start == keys_start => {}
            Some(_) => return Err(MerkleRadixTrieError::NonMatchingChunk),
            None => return Err(MerkleRadixTrieError::TreeAlreadyComplete),
        }
        if let Some(keys_end) = &chunk.keys_end {
            if keys_start > *keys_end {
                return Err(MerkleRadixTrieError::InvalidChunk("invalid keys"));
            }
        }
        if !chunk.items.is_empty() {
            if keys_start > chunk.items[0].0 {
                return Err(MerkleRadixTrieError::InvalidChunk(
                    "first key is inconsistent with key range",
                ));
            }
            if let Some(keys_end) = &chunk.keys_end {
                if chunk.items.last().unwrap().0 >= *keys_end {
                    return Err(MerkleRadixTrieError::InvalidChunk(
                        "last key is inconsistent with key range",
                    ));
                }
            }
            for i in 0..chunk.items.len() - 1 {
                if chunk.items[i].0 >= chunk.items[i + 1].0 {
                    return Err(MerkleRadixTrieError::InvalidChunk("items are not sorted"));
                }
            }
        }
        if !chunk.proof.verify(&expected_hash) {
            return Err(MerkleRadixTrieError::InvalidChunk(
                "last item proof isn't valid",
            ));
        }
        // We only have a valid end if it's the proven node itself, or if it's the direct child of
        // one of the nodes of the proof.
        if let Some(keys_end) = &chunk.keys_end {
            let mut is_valid_end = false;
            if let Some(last_proof_node) = chunk.proof.nodes.first() {
                if last_proof_node.key == *keys_end {
                    is_valid_end = true;
                }
            }
            if !is_valid_end {
                for proof_node in &chunk.proof.nodes {
                    if proof_node.key.len() + 1 == keys_end.len()
                        && proof_node.key.is_prefix_of(keys_end)
                    {
                        is_valid_end = true;
                        break;
                    }
                }
            }
            if !is_valid_end {
                return Err(MerkleRadixTrieError::InvalidChunk("invalid end key"));
            }
            if let Some(last_proof_node) = chunk.proof.nodes.first() {
                if last_proof_node.key >= *keys_end {
                    return Err(MerkleRadixTrieError::InvalidChunk(
                        "end key inconsistent with last proof node",
                    ));
                }
            }
        }

        // First, clear the tree's stumps.
        self.clear_stumps(txn, keys_start..);
        // Then, put all the new items.
        let missing_range = chunk.keys_end.clone().map(|end| end..);
        for (key, value) in chunk.items {
            self.put_raw(txn, &key, value, &missing_range);
        }

        // Mark the remaining stumps.
        if let Some(keys_end) = &chunk.keys_end {
            self.mark_stumps(txn, keys_end.clone().., &chunk.proof.nodes)?;
        }

        // Update the hashes and check that we're good.
        let actual_hash = self.update_hashes(txn, &KeyNibbles::ROOT, &missing_range);

        if actual_hash != expected_hash {
            error!(
                "Putting chunk failed, have hash {} wanted hash {}",
                actual_hash, expected_hash
            );
            return Err(MerkleRadixTrieError::ChunkHashMismatch);
        }

        let mut root_node = self.get_root(txn).unwrap();
        root_node.root_data.as_mut().unwrap().incomplete = chunk.keys_end.clone().map(|end| end..);
        self.put_node(txn, &root_node);

        Ok(())
    }

    /// `keys_start` is inclusive and marks the first key to be removed.
    pub fn remove_chunk(
        &self,
        txn: &mut WriteTransaction,
        keys_start: KeyNibbles,
    ) -> Result<(), MerkleRadixTrieError> {
        let missing_range = self.get_missing_range(txn);

        // PITODO: Optimize this.
        // Currently, we need the whole list of remaining nodes to generate the chunk proof,
        // and the remaining list of nodes to remove those.
        let to_remove: Vec<_> = self
            .get_trie_chunk(
                txn,
                &keys_start,
                (self.num_leaves(txn) + self.num_hybrids(txn)) as usize, //PITODO: optimize this
            )
            .drain(..)
            .map(|node| node.key)
            .collect();

        if to_remove.is_empty() {
            return Ok(());
        }

        // Next, we get a proof for the last remaining leaf.
        let proof = if let Some(last_remaining) = self.get_predecessor(txn, &keys_start) {
            self.get_proof(txn, vec![last_remaining])
                .ok_or(MerkleRadixTrieError::InvalidChunk(
                    "Failed to generate proof",
                ))?
        } else {
            TrieProof::new(vec![self.get_root(txn).unwrap()])
        };

        // Then, clear the tree's stumps.
        if let Some(ref missing_range) = missing_range {
            self.clear_stumps(txn, missing_range.clone());
        }

        // Then, remove all the items.
        for key in to_remove {
            self.remove(txn, &key);
        }

        // Mark the remaining stumps.
        self.mark_stumps(txn, keys_start.clone().., &proof.nodes)?;

        let mut root_node = self.get_root(txn).unwrap();
        root_node.root_data.as_mut().unwrap().incomplete = Some(keys_start..);
        self.put_node(txn, &root_node);

        Ok(())
    }

    /// Removes the value in the Merkle Radix Trie at the given key. If the key doesn't exist
    /// then this function just returns silently. You can't use this to check the existence of a
    /// given prefix.
    pub fn remove(&self, txn: &mut WriteTransaction, key: &KeyNibbles) {
        // PITODO: add result type
        // Start by getting the root node.
        let mut cur_node = self
            .get_root(txn)
            .expect("Patricia Trie must have a root node!");

        let missing_range = cur_node
            .root_data
            .clone()
            .expect("root node needs root data")
            .incomplete;

        // And initialize the root path.
        let mut root_path: Vec<TrieNode> = vec![];

        // Go down the trie until you find the key.
        loop {
            // If the current node key is no longer a prefix for the given key then the key doesn't
            // exist and we stop here.
            if !cur_node.key.is_prefix_of(key) {
                return;
            }

            // If the current node key is equal to our given key, we have found our node.
            // Update/remove the node.
            if cur_node.key == *key {
                // Remove the value from the node.
                // PITODO check if hybrid or leaf node
                cur_node.value = None;
                if cur_node.is_root() || cur_node.has_children() {
                    // Node was a hybrid node and is now a branch node.
                    let num_children = cur_node.iter_children().count();

                    // If it has only a single child and isn't the root node, merge it with that child.
                    let count_updates = if num_children == 1 && !cur_node.is_root() {
                        // Remove the node from the database.
                        txn.remove(&self.db, &cur_node.key);

                        // Get the node's only child and add it to the root path.
                        let only_child_key = cur_node
                            .iter_children()
                            .next()
                            .unwrap()
                            .key(&cur_node.key, &missing_range)
                            .expect("no stump");

                        let only_child = self.get_node(txn, &only_child_key).unwrap();

                        root_path.push(only_child);

                        // We removed a hybrid node.
                        CountUpdates {
                            hybrids: -1,
                            ..Default::default()
                        }
                    } else {
                        // The node is root or has multiple children, thus we cannot remove it.
                        // Instead we must convert it to a branch node by removing its value.

                        // Update the node and add it to the root path.
                        self.put_node(txn, &cur_node);

                        root_path.push(cur_node);

                        // We converted a hybrid node into a branch node.
                        CountUpdates {
                            branches: 1,
                            hybrids: -1,
                            ..Default::default()
                        }
                    };

                    // Update the keys and hashes of the rest of the root path.
                    self.update_keys(txn, root_path, count_updates);

                    return;
                } else {
                    // Node was a leaf node, delete if from the database.
                    txn.remove(&self.db, key);
                    break;
                }
            }

            // Try to find a child of the current node that matches our key.
            match cur_node.child_key(key, &missing_range) {
                // If no matching child exists, then the key doesn't exist and we stop here.
                Err(_) => {
                    return;
                }
                // If there's a child, then we update the current node and the root path, and
                // continue down the trie.
                Ok(child_key) => {
                    root_path.push(cur_node);
                    cur_node = self.get_node(txn, &child_key).unwrap();
                }
            }
        }

        // Walk along the root path towards the root node, starting with the immediate predecessor
        // of the node with the given key, and update the nodes along the way.
        let mut child_key = key.clone();
        let mut count_updates = CountUpdates {
            leaves: -1,
            ..Default::default()
        };

        while let Some(mut parent_node) = root_path.pop() {
            // Remove the child from the parent node.
            let prev_parent_kind = parent_node.kind();
            parent_node.remove_child(&child_key).unwrap();
            count_updates.apply_update(prev_parent_kind, parent_node.kind());

            // Get the number of children of the node.
            let num_children = parent_node.iter_children().count();

            // If the node has only a single child (and it isn't the root node), merge it with the
            // child.
            if num_children == 1 && !parent_node.is_root() && !parent_node.is_hybrid() {
                // Remove the node from the database.
                txn.remove(&self.db, &parent_node.key);
                count_updates.apply_update(parent_node.kind(), None);

                // Get the node's only child and add it to the root path.
                let only_child_key = parent_node
                    .iter_children()
                    .next()
                    .unwrap()
                    .key(&parent_node.key, &missing_range)
                    .expect("no stump");

                let only_child = self.get_node(txn, &only_child_key).unwrap();

                root_path.push(only_child);

                // Update the keys and hashes of the rest of the root path.
                self.update_keys(txn, root_path, count_updates);

                return;
            }
            // If the node has any children, or it is either the root or a hybrid node, we just store the
            // parent node in the database and the root path. Then we update the keys and hashes of
            // of the root path.
            else if !parent_node.is_empty() {
                self.put_node(txn, &parent_node);

                root_path.push(parent_node);

                self.update_keys(txn, root_path, count_updates);

                return;
            }
            // Otherwise, our node must have no children and not be the root/hybrid node. In this case we
            // need to remove it too, so we loop again.
            else {
                txn.remove(&self.db, &parent_node.key);
                count_updates.apply_update(parent_node.kind(), None);
                child_key = parent_node.key.clone();
            }
        }
    }

    /// Produces a Merkle proof of the inclusion of the given keys in the Merkle Radix Trie. The
    /// proof consists of the path from the leaves that we want to prove inclusion all the way up
    /// to the root. For example, for the following trie:
    ///              R
    ///              |
    ///              B1
    ///          /   |   \
    ///        B2   L3   B3
    ///       / \        / \
    ///      L1 L2      L4 L5
    /// If we want a proof for the nodes L1 and L3, the proof will consist of the nodes L1, B2, L3,
    /// B1 and R. Note that:
    ///     1. Unlike Merkle proofs we don't need the adjacent branch nodes. That's because our
    ///        branch nodes already include the hashes of its children.
    ///     2. The nodes are always returned in post-order.
    /// If any of the given keys doesn't exist this function just returns None.
    /// The exclusion (non-inclusion) of keys in the Merkle Radix Trie could also be proven, but it
    /// requires some light refactoring to the way proofs work.
    pub fn get_proof<T>(&self, txn: &Transaction, mut keys: Vec<T>) -> Option<TrieProof>
    where
        T: Borrow<KeyNibbles> + Ord + Display,
    {
        // We sort the keys to simplify traversal in post-order.
        keys.sort();

        // Initialize the vector that will contain the proof.
        let mut proof_nodes = Vec::new();

        // Initialize the pointer node, we will use it to go up and down the tree. We always start
        // at the root.
        let mut pointer_node = self
            .get_root(txn)
            .expect("Merkle Radix Trie must have a root node!");

        // Initialize the root path.
        let mut root_path: Vec<TrieNode> = vec![];

        // Get the first key.
        let mut cur_key = keys
            .pop()
            .expect("There must be at least one key to prove!");

        let missing_range = self.get_missing_range(txn);

        // Iterate over all the keys that we wish to prove.
        loop {
            // Go down the trie until we find a node with our key or we can't go any further.
            loop {
                // If the key does not match, the requested key is not part of this trie. In
                // this case, we can't produce a proof so we terminate now.
                if !pointer_node.key.is_prefix_of(cur_key.borrow()) {
                    error!(
                        "Pointer node with key {} is not a prefix to the current node with key {}.",
                        pointer_node.key, cur_key
                    );
                    return None;
                }

                // If the key fully matches, we have found the requested node. We must check that
                // it is a leaf/hybrid node, we don't want to prove branch nodes.
                if &pointer_node.key == cur_key.borrow() {
                    if pointer_node.value.is_none() {
                        error!(
                            "Pointer node with key {} is a branch node. We don't want to prove branch nodes.",
                            pointer_node.key,
                        );
                        return None;
                    }

                    break;
                }

                // Otherwise, try to find a child of the pointer node that matches our key.
                match pointer_node.child_key(cur_key.borrow(), &missing_range) {
                    // If no matching child exists, then the requested key is not part of this
                    // trie. Once again, we can't produce a proof so we terminate now.
                    Err(_) => {
                        error!(
                            "Key {} is not a part of the trie. Can't produce the proof.",
                            cur_key
                        );
                        return None;
                    }
                    // If there's a child, then we update the pointer node and the root path, and
                    // continue down the trie.
                    Ok(child_key) => {
                        let old_pointer_node = mem::replace(
                            &mut pointer_node,
                            self.get_node(txn, &child_key).unwrap(),
                        );
                        root_path.push(old_pointer_node);
                    }
                }
            }

            // Get the next key. If there's no next key then we get out of the loop.
            match keys.pop() {
                None => {
                    // Add the remaining nodes in the root path to the proof. Evidently they must
                    // be added in the reverse order.
                    proof_nodes.push(pointer_node);
                    root_path.reverse();
                    proof_nodes.append(&mut root_path);

                    // Exit the loop.
                    break;
                }
                Some(key) => cur_key = key,
            }

            // Go up the root path until we get to a node that is a prefix to our current key.
            // Add the nodes you remove to the proof.
            while !pointer_node.key.is_prefix_of(cur_key.borrow()) {
                let old_pointer_node = mem::replace(
                    &mut pointer_node,
                    root_path
                        .pop()
                        .expect("Root path must contain at least the root node!"),
                );

                proof_nodes.push(old_pointer_node);
            }
        }

        // Return the proof.
        Some(TrieProof::new(proof_nodes))
    }

    /// Creates a proof for the chunk of the Merkle Radix Trie that starts at the key `start` (which
    /// might or not be a part of the trie, if it is then it will be part of the chunk) and contains
    /// at most `size` leaf nodes.
    /// PITODO: Perhaps remove this?
    pub fn get_chunk_proof(
        &self,
        txn: &Transaction,
        start: &KeyNibbles,
        size: usize,
    ) -> Option<TrieProof> {
        let chunk = self.get_trie_chunk(txn, start, size);

        let chunk_keys = chunk.iter().map(|node| &node.key).collect();

        self.get_proof(txn, chunk_keys)
    }

    pub fn update_root(&self, txn: &mut WriteTransaction) {
        let missing_range = self.get_missing_range(txn);
        self.update_hashes(txn, &KeyNibbles::ROOT, &missing_range);
    }

    fn check_within_complete_part(
        &self,
        txn: &Transaction,
        key: &KeyNibbles,
    ) -> Result<Option<RangeFrom<KeyNibbles>>, IncompleteTrie> {
        let missing_range = self.get_missing_range(txn);
        let is_in_complete_part = missing_range
            .as_ref()
            .map(|r| !r.contains(key))
            .unwrap_or(true);
        if is_in_complete_part {
            Ok(missing_range)
        } else {
            Err(IncompleteTrie)
        }
    }

    /// Returns the range of missing keys in the partial tree.
    /// If the tree is complete, it returns `None`.
    pub fn get_missing_range(&self, txn: &Transaction) -> Option<RangeFrom<KeyNibbles>> {
        self.get_root(txn)
            .expect("trie needs root node")
            .root_data
            .expect("root node needs root data")
            .incomplete
    }

    /// Returns the root node, if there is one.
    fn get_root(&self, txn: &Transaction) -> Option<TrieNode> {
        self.get_node(txn, &KeyNibbles::ROOT)
    }

    /// Updates the keys for a chain of nodes and marks those nodes as dirty. It assumes that the
    /// path starts at the root node and that each consecutive node is a child of the previous node.
    fn update_keys(
        &self,
        txn: &mut WriteTransaction,
        mut root_path: Vec<TrieNode>,
        count_updates: CountUpdates,
    ) {
        {
            // Save it directly if the root node doesn't have to be updated for other reasons.
            let only_root_needs_update = root_path.len() == 1;
            let root = root_path.first_mut().expect("Root path must not be empty");
            if !count_updates.is_empty() {
                let root_data = root
                    .root_data
                    .as_mut()
                    .expect("Root path must start with a root node");
                count_updates.update_root_data(root_data);
                if only_root_needs_update {
                    self.put_node(txn, root);
                    return;
                }
            }
        }

        // Get the first node in the path.
        let mut child_node = root_path.pop().expect("Root path must not be empty!");

        // Go up the root path until you get to the root.
        while let Some(mut parent_node) = root_path.pop() {
            // Update and store the parent node.
            parent_node
                // Mark this node as dirty by storing the default hash.
                // Also updates the child key.
                .put_child(&child_node.key, Blake2bHash::default())
                .unwrap();
            assert_eq!(root_path.is_empty(), parent_node.is_root(),);
            self.put_node(txn, &parent_node);

            child_node = parent_node;
        }
    }

    /// Updates the hashes of all dirty nodes in the subtree specified by `key`.
    fn update_hashes(
        &self,
        txn: &mut WriteTransaction,
        key: &KeyNibbles,
        missing_range: &Option<RangeFrom<KeyNibbles>>,
    ) -> Blake2bHash {
        let mut node: TrieNode = self.get_node(txn, key).unwrap();
        if !node.has_children() {
            return node.hash_assert_complete();
        }

        // Compute sub hashes if necessary.
        for child in node.iter_children_mut() {
            if !child.has_hash() {
                // TODO This could be parallelized.
                child.hash = self.update_hashes(
                    txn,
                    &child.key(key, missing_range).expect("no stump"),
                    missing_range,
                );
            }
        }
        self.put_node(txn, &node);
        node.hash_assert_complete()
    }

    /// Returns the last key containing a value before the given key.
    fn get_predecessor(&self, txn: &Transaction, key: &KeyNibbles) -> Option<KeyNibbles> {
        let mut predecessor_branch = None;
        let mut cur_node = self
            .get_root(txn)
            .expect("The Merkle Radix Trie didn't have a root node!");

        let missing_range = cur_node
            .root_data
            .clone()
            .expect("root node needs root data")
            .incomplete;

        // First, find the node.
        loop {
            // If the current node key is equal to or larger than the given key, we must have
            // branched off earlier for a predecessor.
            if cur_node.key >= *key {
                break;
            }

            // The current node is a potential predecessor if it has a value.
            if cur_node.value.is_some() {
                predecessor_branch = Some((cur_node.key.clone(), false));
            }

            // Try to find a child of the current node that matches our key.
            let child_index = cur_node.child_index(key).unwrap_or(16);
            // Every index before that is a potential predecessor.
            for i in (0..child_index).rev() {
                if let Some(child) = &cur_node.children[i] {
                    predecessor_branch = Some((
                        child.key(&cur_node.key, &missing_range).expect("no stump"),
                        true,
                    ));
                    break;
                }
            }

            // If we're already off the path to our key, we're done.
            if child_index == 16 {
                break;
            }
            match &cur_node.children[child_index] {
                // If no matching child exists, we're done.
                None => break,
                // If there's a child, then we update the current node and the root path, and
                // continue down the trie.
                Some(child) => {
                    cur_node = self
                        .get_node(
                            txn,
                            &child.key(&cur_node.key, &missing_range).expect("no stump"),
                        )
                        .unwrap();
                }
            }
        }

        let (mut cur_node, need_to_go_down) = match predecessor_branch {
            Some((key, need_to_go_down)) => (self.get_node(txn, &key).unwrap(), need_to_go_down),
            None => return None,
        };

        if need_to_go_down {
            // Now continue trying to get the rightmost child.
            while let Some(child) = cur_node.children.iter().rev().flatten().next() {
                cur_node = self
                    .get_node(
                        txn,
                        &child.key(&cur_node.key, &missing_range).expect("no stump"),
                    )
                    .unwrap();
            }
        }
        assert!(cur_node.value.is_some());
        Some(cur_node.key)
    }

    /// Returns the nodes of the chunk of the Merkle Radix Trie that starts at the key `start` and
    /// has size `size`. This is used by the `get_chunk` and `get_chunk_proof` functions.
    fn get_trie_chunk(&self, txn: &Transaction, start: &KeyNibbles, size: usize) -> Vec<TrieNode> {
        let mut chunk = Vec::new();

        if size == 0 {
            return chunk;
        }

        let root = self
            .get_root(txn)
            .expect("The Merkle Radix Trie didn't have a root node!");
        let missing_range = root
            .root_data
            .clone()
            .expect("root node needs root data")
            .incomplete;

        let mut stack = vec![root];

        while let Some(item) = stack.pop() {
            for child in item.iter_children().rev() {
                // We are not including stumps in our chunk.
                let combined = match child.key(&item.key, &missing_range) {
                    Ok(key) => key,
                    Err(MerkleRadixTrieError::ChildIsStump) => continue,
                    Err(e) => unreachable!("Unexpected behavior when getting chunk: {}", e),
                };

                if combined.is_prefix_of(start) || *start <= combined {
                    stack.push(self.get_node(txn, &combined)
                        .expect("Failed to find the child of a Merkle Radix Trie node. The database must be corrupt!"));
                }
            }

            if item.value.is_some() {
                if start.len() < item.key.len() || *start <= item.key {
                    chunk.push(item);
                }
                if chunk.len() >= size {
                    break;
                }
            }
        }

        chunk
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nimiq_test_log::test;

    #[test]
    fn get_put_remove_works() {
        let key_1 = "413f22b3e".parse().unwrap();
        let key_2 = "413b39931".parse().unwrap();
        let key_3 = "413b397fa".parse().unwrap();
        let key_4 = "cfb986f5a".parse().unwrap();

        let env = nimiq_database::volatile::VolatileEnvironment::new(10).unwrap();
        let trie = MerkleRadixTrie::new(env.clone(), "database");
        let mut txn = WriteTransaction::new(&env);

        assert_eq!(trie.count_nodes(&txn), (0, 0, 0));

        trie.put(&mut txn, &key_1, 80085).expect("complete trie");
        assert_eq!(trie.count_nodes(&txn), (0, 0, 1));
        trie.put(&mut txn, &key_2, 999).expect("complete trie");
        assert_eq!(trie.count_nodes(&txn), (1, 0, 2));
        trie.put(&mut txn, &key_3, 1337).expect("complete trie");

        assert_eq!(trie.count_nodes(&txn), (2, 0, 3));
        assert_eq!(trie.get(&txn, &key_1).expect("complete trie"), Some(80085));
        assert_eq!(trie.get(&txn, &key_2).expect("complete trie"), Some(999));
        assert_eq!(trie.get(&txn, &key_3).expect("complete trie"), Some(1337));
        assert_eq!(trie.get(&txn, &key_4).expect("complete trie"), None::<i32>);

        trie.remove(&mut txn, &key_4);
        assert_eq!(trie.count_nodes(&txn), (2, 0, 3));
        assert_eq!(trie.get(&txn, &key_1).expect("complete trie"), Some(80085));
        assert_eq!(trie.get(&txn, &key_2).expect("complete trie"), Some(999));
        assert_eq!(trie.get(&txn, &key_3).expect("complete trie"), Some(1337));

        trie.remove(&mut txn, &key_1);
        assert_eq!(trie.count_nodes(&txn), (1, 0, 2));
        assert_eq!(trie.get(&txn, &key_1).expect("complete trie"), None::<i32>);
        assert_eq!(trie.get(&txn, &key_2).expect("complete trie"), Some(999));
        assert_eq!(trie.get(&txn, &key_3).expect("complete trie"), Some(1337));

        trie.remove(&mut txn, &key_2);
        assert_eq!(trie.count_nodes(&txn), (0, 0, 1));
        assert_eq!(trie.get(&txn, &key_1).expect("complete trie"), None::<i32>);
        assert_eq!(trie.get(&txn, &key_2).expect("complete trie"), None::<i32>);
        assert_eq!(trie.get(&txn, &key_3).expect("complete trie"), Some(1337));

        trie.remove(&mut txn, &key_3);
        assert_eq!(trie.count_nodes(&txn), (0, 0, 0));
        assert_eq!(trie.get(&txn, &key_1).expect("complete trie"), None::<i32>);
        assert_eq!(trie.get(&txn, &key_2).expect("complete trie"), None::<i32>);
        assert_eq!(trie.get(&txn, &key_3).expect("complete trie"), None::<i32>);
    }

    #[test]
    fn get_proof_works() {
        //          |
        //         cfb98
        //        /     |
        //       6    e0f6
        //      /  \
        //   ab9   f5a

        let key_1 = "cfb986f5a".parse().unwrap();
        let key_2 = "cfb986ab9".parse().unwrap();
        let key_3 = "cfb98e0f6".parse().unwrap();
        let key_4 = "cfb98e0f5".parse().unwrap();

        let env = nimiq_database::volatile::VolatileEnvironment::new(10).unwrap();
        let trie = MerkleRadixTrie::new(env.clone(), "database");
        let mut txn = WriteTransaction::new(&env);

        trie.put(&mut txn, &key_1, 9).expect("complete trie");
        trie.put(&mut txn, &key_2, 8).expect("complete trie");
        trie.put(&mut txn, &key_3, 7).expect("complete trie");
        trie.update_root(&mut txn);

        let proof = trie.get_proof(&txn, vec![&key_1, &key_2, &key_3]).unwrap();
        assert_eq!(proof.nodes.len(), 6);
        assert_eq!(proof.verify(&trie.root_hash(&txn)), true);

        let proof = trie.get_proof(&txn, vec![&key_3, &key_1, &key_2]).unwrap();
        assert_eq!(proof.nodes.len(), 6);
        assert_eq!(proof.verify(&trie.root_hash(&txn)), true);

        let proof = trie.get_proof(&txn, vec![&key_1, &key_3]).unwrap();
        assert_eq!(proof.nodes.len(), 5);
        assert_eq!(proof.verify(&trie.root_hash(&txn)), true);

        let proof = trie.get_proof(&txn, vec![&key_1, &key_2]).unwrap();
        assert_eq!(proof.nodes.len(), 5);
        assert_eq!(proof.verify(&trie.root_hash(&txn)), true);

        let proof = trie.get_proof(&txn, vec![&key_1]).unwrap();
        assert_eq!(proof.nodes.len(), 4);
        assert_eq!(proof.verify(&trie.root_hash(&txn)), true);

        let proof = trie.get_proof(&txn, vec![&key_3]).unwrap();
        assert_eq!(proof.nodes.len(), 3);
        assert_eq!(proof.verify(&trie.root_hash(&txn)), true);

        let proof = trie.get_proof(&txn, vec![&key_4, &key_2]);
        assert!(proof.is_none());

        let proof = trie.get_proof(&txn, vec![&key_4]);
        assert!(proof.is_none());
    }

    #[test]
    fn get_chunk_works() {
        let key_1 = "cfb986f5a".parse().unwrap();
        let key_2 = "cfb986ab9".parse().unwrap();
        let key_3 = "cfb98e0f6".parse().unwrap();
        let key_4 = "cfb98e0f5".parse().unwrap();

        let env = nimiq_database::volatile::VolatileEnvironment::new(10).unwrap();
        let trie = MerkleRadixTrie::new(env.clone(), "database");
        let mut txn = WriteTransaction::new(&env);

        trie.put(&mut txn, &key_1, 9).expect("complete trie");
        trie.put(&mut txn, &key_2, 8).expect("complete trie");
        trie.put(&mut txn, &key_3, 7).expect("complete trie");
        trie.update_root(&mut txn);

        let chunk = trie.get_chunk_proof(&txn, &KeyNibbles::ROOT, 100).unwrap();
        assert_eq!(chunk.nodes.len(), 6);
        assert_eq!(chunk.verify(&trie.root_hash(&txn)), true);

        let chunk = trie.get_chunk_proof(&txn, &KeyNibbles::ROOT, 3).unwrap();
        assert_eq!(chunk.nodes.len(), 6);
        assert_eq!(chunk.verify(&trie.root_hash(&txn)), true);

        let chunk = trie.get_chunk_proof(&txn, &KeyNibbles::ROOT, 2).unwrap();
        assert_eq!(chunk.nodes.len(), 5);
        assert_eq!(chunk.verify(&trie.root_hash(&txn)), true);

        let chunk = trie.get_chunk_proof(&txn, &key_3, 100).unwrap();
        assert_eq!(chunk.nodes.len(), 3);
        assert_eq!(chunk.verify(&trie.root_hash(&txn)), true);

        let chunk = trie.get_chunk_proof(&txn, &key_4, 100).unwrap();
        assert_eq!(chunk.nodes.len(), 3);
        assert_eq!(chunk.verify(&trie.root_hash(&txn)), true);
    }

    #[test]
    fn hybrid_nodes_work() {
        let key_1 = "413f22".parse().unwrap();
        let key_2 = "413".parse().unwrap();
        let key_3 = "413f227fa".parse().unwrap();
        let key_4 = "413b391".parse().unwrap();
        let key_5 = "412324".parse().unwrap();

        let env = nimiq_database::volatile::VolatileEnvironment::new(10).unwrap();
        let trie = MerkleRadixTrie::new(env.clone(), "database");
        let mut txn = WriteTransaction::new(&env);

        let initial_hash = trie.root_hash(&txn);
        assert_eq!(trie.count_nodes(&txn), (0, 0, 0));
        trie.put(&mut txn, &key_1, 80085).expect("complete trie");
        assert_eq!(trie.count_nodes(&txn), (0, 0, 1));
        trie.put(&mut txn, &key_2, 999).expect("complete trie");
        assert_eq!(trie.count_nodes(&txn), (0, 1, 1));
        trie.put(&mut txn, &key_3, 1337).expect("complete trie");
        assert_eq!(trie.count_nodes(&txn), (0, 2, 1));
        trie.put(&mut txn, &key_4, 6969).expect("complete trie");

        assert_eq!(trie.count_nodes(&txn), (0, 2, 2));
        assert_eq!(trie.get(&txn, &key_1).expect("complete trie"), Some(80085));
        assert_eq!(trie.get(&txn, &key_2).expect("complete trie"), Some(999));
        assert_eq!(trie.get(&txn, &key_3).expect("complete trie"), Some(1337));
        assert_eq!(trie.get(&txn, &key_4).expect("complete trie"), Some(6969));
        assert_eq!(trie.get(&txn, &key_5).expect("complete trie"), None::<i32>);

        trie.remove(&mut txn, &key_5);
        assert_eq!(trie.count_nodes(&txn), (0, 2, 2));
        assert_eq!(trie.get(&txn, &key_1).expect("complete trie"), Some(80085));
        assert_eq!(trie.get(&txn, &key_2).expect("complete trie"), Some(999));
        assert_eq!(trie.get(&txn, &key_3).expect("complete trie"), Some(1337));
        assert_eq!(trie.get(&txn, &key_4).expect("complete trie"), Some(6969));

        trie.remove(&mut txn, &key_1);
        assert_eq!(trie.count_nodes(&txn), (0, 1, 2));
        assert_eq!(trie.get(&txn, &key_1).expect("complete trie"), None::<i32>);
        assert_eq!(trie.get(&txn, &key_2).expect("complete trie"), Some(999));
        assert_eq!(trie.get(&txn, &key_3).expect("complete trie"), Some(1337));
        assert_eq!(trie.get(&txn, &key_4).expect("complete trie"), Some(6969));

        trie.remove(&mut txn, &key_2);
        assert_eq!(trie.count_nodes(&txn), (1, 0, 2));
        assert_eq!(trie.get(&txn, &key_1).expect("complete trie"), None::<i32>);
        assert_eq!(trie.get(&txn, &key_2).expect("complete trie"), None::<i32>);
        assert_eq!(trie.get(&txn, &key_3).expect("complete trie"), Some(1337));
        assert_eq!(trie.get(&txn, &key_4).expect("complete trie"), Some(6969));

        trie.remove(&mut txn, &key_3);
        assert_eq!(trie.count_nodes(&txn), (0, 0, 1));
        assert_eq!(trie.get(&txn, &key_1).expect("complete trie"), None::<i32>);
        assert_eq!(trie.get(&txn, &key_2).expect("complete trie"), None::<i32>);
        assert_eq!(trie.get(&txn, &key_3).expect("complete trie"), None::<i32>);
        assert_eq!(trie.get(&txn, &key_4).expect("complete trie"), Some(6969));

        trie.remove(&mut txn, &key_4);
        assert_eq!(trie.count_nodes(&txn), (0, 0, 0));
        assert_eq!(trie.get(&txn, &key_1).expect("complete trie"), None::<i32>);
        assert_eq!(trie.get(&txn, &key_2).expect("complete trie"), None::<i32>);
        assert_eq!(trie.get(&txn, &key_3).expect("complete trie"), None::<i32>);
        assert_eq!(trie.get(&txn, &key_4).expect("complete trie"), None::<i32>);

        assert_eq!(trie.root_hash(&txn), initial_hash);
    }

    #[test]
    fn get_predecessor_works() {
        let key_1 = "413f22".parse().unwrap();
        let key_2 = "413".parse().unwrap();
        let key_3 = "413f227fa".parse().unwrap();
        let key_4 = "413b391".parse().unwrap();
        let key_5 = "412324".parse().unwrap();
        let key_6 = "413f227fb".parse().unwrap();
        let key_7 = "413f227fa0".parse().unwrap();

        let env = nimiq_database::volatile::VolatileEnvironment::new(10).unwrap();
        let trie = MerkleRadixTrie::new(env.clone(), "database");
        let mut txn = WriteTransaction::new(&env);

        assert_eq!(trie.get_predecessor(&txn, &KeyNibbles::ROOT), None);
        assert_eq!(trie.get_predecessor(&txn, &key_1), None);
        assert_eq!(trie.get_predecessor(&txn, &key_2), None);

        trie.put(&mut txn, &key_1, 80085).expect("complete trie");
        trie.put(&mut txn, &key_2, 999).expect("complete trie");
        trie.put(&mut txn, &key_3, 1337).expect("complete trie");
        trie.put(&mut txn, &key_4, 6969).expect("complete trie");
        assert_eq!(trie.count_nodes(&txn), (0, 2, 2));

        assert_eq!(trie.get_predecessor(&txn, &KeyNibbles::ROOT), None);
        assert_eq!(trie.get_predecessor(&txn, &key_1), Some(key_4.clone()));
        assert_eq!(trie.get_predecessor(&txn, &key_2), None);
        assert_eq!(trie.get_predecessor(&txn, &key_3), Some(key_1.clone()));
        assert_eq!(trie.get_predecessor(&txn, &key_4), Some(key_2.clone()));
        assert_eq!(trie.get_predecessor(&txn, &key_5), None);
        assert_eq!(trie.get_predecessor(&txn, &key_6), Some(key_3.clone()));
        assert_eq!(trie.get_predecessor(&txn, &key_7), Some(key_3.clone()));
    }

    #[test]
    fn test_keys_end_on_partial_tree() {
        let key_1: KeyNibbles = "5".parse().unwrap();
        let key_2: KeyNibbles = "413b39931".parse().unwrap();
        let key_3: KeyNibbles = "4".parse().unwrap();

        let proof_value_2 = TrieNode::new_leaf(key_2.clone(), vec![99]);
        let mut proof_root = TrieNode::new_root();
        proof_root
            .put_child(&proof_value_2.key, proof_value_2.hash_assert_complete())
            .unwrap();

        let env = nimiq_database::volatile::VolatileEnvironment::new(10).unwrap();
        let trie = MerkleRadixTrie::new_incomplete(env.clone(), "database");
        let mut txn = WriteTransaction::new(&env);
        assert!(!trie.is_complete(&txn));

        trie.put_chunk(
            &mut txn,
            KeyNibbles::ROOT,
            TrieChunk::new(
                Some(key_3.clone()),
                Vec::new(),
                TrieProof::new(vec![TrieNode::new_root()]),
            ),
            TrieNode::new_root().hash_assert_complete(),
        )
        .unwrap();
        assert_eq!(trie.count_nodes(&txn), (0, 0, 0));
        assert!(!trie.is_complete(&txn));

        trie.put_chunk(
            &mut txn,
            key_3.clone(),
            TrieChunk::new(
                Some(key_3.clone()),
                Vec::new(),
                TrieProof::new(vec![proof_root.clone()]),
            ),
            proof_root.hash_assert_complete(),
        )
        .unwrap();
        assert_eq!(trie.count_nodes(&txn), (0, 0, 0));
        assert!(!trie.is_complete(&txn));

        trie.put_chunk(
            &mut txn,
            key_3.clone(),
            TrieChunk::new(
                Some(key_1.clone()),
                vec![(key_2.clone(), vec![99])],
                TrieProof::new(vec![proof_value_2.clone(), proof_root.clone()]),
            ),
            proof_root.hash_assert_complete(),
        )
        .unwrap();
        assert_eq!(trie.count_nodes(&txn), (0, 0, 1));
        assert!(!trie.is_complete(&txn));

        trie.put_chunk(
            &mut txn,
            key_1.clone(),
            TrieChunk::new(
                None,
                Vec::new(),
                TrieProof::new(vec![proof_value_2.clone(), proof_root.clone()]),
            ),
            proof_root.hash_assert_complete(),
        )
        .unwrap();
        assert_eq!(trie.count_nodes(&txn), (0, 0, 1));
        assert!(trie.is_complete(&txn));
    }

    #[test]
    fn get_chunk_with_proof_works() {
        let key_1 = "413f22".parse().unwrap();
        let key_2 = "413".parse().unwrap();
        let key_3 = "413f227fa".parse().unwrap();
        let key_4 = "413b391".parse().unwrap();
        let key_5: KeyNibbles = "412324".parse().unwrap();

        let env = nimiq_database::volatile::VolatileEnvironment::new(10).unwrap();
        let original = MerkleRadixTrie::new(env.clone(), "original");
        let mut txn = WriteTransaction::new(&env);

        original
            .put(&mut txn, &key_1, 80085)
            .expect("complete trie");
        original.put(&mut txn, &key_2, 999).expect("complete trie");
        original.put(&mut txn, &key_3, 1337).expect("complete trie");
        original.put(&mut txn, &key_4, 6969).expect("complete trie");
        assert_eq!(original.count_nodes(&txn), (0, 2, 2));

        for i in 0..10 {
            original.get_chunk_with_proof(&txn, KeyNibbles::ROOT.., i);
            original.get_chunk_with_proof(&txn, key_1.clone().., i);
            original.get_chunk_with_proof(&txn, key_2.clone().., i);
            original.get_chunk_with_proof(&txn, key_3.clone().., i);
            original.get_chunk_with_proof(&txn, key_4.clone().., i);
            original.get_chunk_with_proof(&txn, key_5.clone().., i);
        }
    }

    #[test]
    fn complete_tree_does_not_accept_chunks() {
        let env = nimiq_database::volatile::VolatileEnvironment::new(10).unwrap();
        let original = MerkleRadixTrie::new(env.clone(), "original");
        let trie = MerkleRadixTrie::new(env.clone(), "copy");
        let mut txn = WriteTransaction::new(&env);

        let hash = original.root_hash(&txn);

        let chunk = original.get_chunk_with_proof(&txn, KeyNibbles::ROOT.., 100);
        assert_eq!(
            trie.put_chunk(&mut txn, KeyNibbles::ROOT, chunk, hash),
            Err(MerkleRadixTrieError::TreeAlreadyComplete)
        );
    }

    #[test]
    fn partial_tree_put_chunks_manual() {
        let key_1 = "413f22".parse().unwrap();
        let key_2 = "413".parse().unwrap();
        let key_3 = "413f227fa".parse().unwrap();
        let key_4 = "413b391".parse().unwrap();
        let key_5 = "412324".parse().unwrap();

        let env = nimiq_database::volatile::VolatileEnvironment::new(10).unwrap();
        let original = MerkleRadixTrie::new(env.clone(), "original");
        let trie = MerkleRadixTrie::new_incomplete(env.clone(), "copy");
        let mut txn = WriteTransaction::new(&env);

        original
            .put(&mut txn, &key_1, 80085)
            .expect("complete trie");
        original.put(&mut txn, &key_2, 999).expect("complete trie");
        original.put(&mut txn, &key_3, 1337).expect("complete trie");
        original.put(&mut txn, &key_4, 6969).expect("complete trie");
        original.update_root(&mut txn);
        assert_eq!(original.count_nodes(&txn), (0, 2, 2));
        assert!(original.is_complete(&txn));

        let hash = original.root_hash(&txn);
        let end = Some(KeyNibbles::ROOT);

        let start = end.unwrap();
        let chunk = original.get_chunk_with_proof(&txn, start.clone().., 0);
        trie.put_chunk(&mut txn, start.clone(), chunk.clone(), hash.clone())
            .unwrap();
        assert_eq!(trie.count_nodes(&txn), (0, 0, 0));
        assert!(!trie.is_complete(&txn));

        let start = chunk.keys_end.unwrap();
        let chunk = original.get_chunk_with_proof(&txn, start.clone().., 1);
        trie.put_chunk(&mut txn, start.clone(), chunk.clone(), hash.clone())
            .unwrap();
        assert_eq!(trie.count_nodes(&txn), (0, 1, 0));
        assert!(!trie.is_complete(&txn));

        let start = chunk.keys_end.unwrap();
        let chunk = original.get_chunk_with_proof(&txn, start.clone().., 0);
        trie.put_chunk(&mut txn, start.clone(), chunk.clone(), hash.clone())
            .unwrap();
        assert_eq!(trie.count_nodes(&txn), (0, 1, 0));
        assert!(!trie.is_complete(&txn));

        let start = chunk.keys_end.unwrap();
        let chunk = original.get_chunk_with_proof(&txn, start.clone().., 2);
        trie.put_chunk(&mut txn, start.clone(), chunk.clone(), hash.clone())
            .unwrap();
        assert_eq!(trie.count_nodes(&txn), (0, 2, 1));
        assert!(!trie.is_complete(&txn));

        let start = chunk.keys_end.unwrap();
        let chunk = original.get_chunk_with_proof(&txn, start.clone().., 2);
        trie.put_chunk(&mut txn, start.clone(), chunk, hash.clone())
            .unwrap();
        assert_eq!(trie.count_nodes(&txn), (0, 2, 2));
        assert!(trie.is_complete(&txn));

        assert_eq!(trie.get(&txn, &key_1).expect("complete trie"), Some(80085));
        assert_eq!(trie.get(&txn, &key_2).expect("complete trie"), Some(999));
        assert_eq!(trie.get(&txn, &key_3).expect("complete trie"), Some(1337));
        assert_eq!(trie.get(&txn, &key_4).expect("complete trie"), Some(6969));
        assert_eq!(trie.get(&txn, &key_5).expect("complete trie"), None::<i32>);
    }

    #[test]
    fn partial_tree_put_chunks_of_different_sizes() {
        let key_1 = "413f22".parse().unwrap();
        let key_2 = "413".parse().unwrap();
        let key_3 = "413f227fa".parse().unwrap();
        let key_4 = "413b391".parse().unwrap();
        let key_5 = "412324".parse().unwrap();

        let env = nimiq_database::volatile::VolatileEnvironment::new(10).unwrap();
        let original = MerkleRadixTrie::new(env.clone(), "original");
        let tries: Vec<_> = (1..5)
            .map(|i| {
                (
                    i,
                    MerkleRadixTrie::new_incomplete(env.clone(), &format!("copy{}", i)),
                )
            })
            .collect();
        let mut txn = WriteTransaction::new(&env);

        original
            .put(&mut txn, &key_1, 80085)
            .expect("complete trie");
        original.put(&mut txn, &key_2, 999).expect("complete trie");
        original.put(&mut txn, &key_3, 1337).expect("complete trie");
        original.put(&mut txn, &key_4, 6969).expect("complete trie");
        original.update_root(&mut txn);
        assert_eq!(original.count_nodes(&txn), (0, 2, 2));
        assert!(original.is_complete(&txn));

        let hash = original.root_hash(&txn);
        for (chunk_size, trie) in tries {
            let mut next_start = KeyNibbles::ROOT;
            for _ in 0..10 {
                let start = next_start;
                let chunk = original.get_chunk_with_proof(&txn, start.clone().., chunk_size);

                trie.put_chunk(&mut txn, start.clone(), chunk.clone(), hash.clone())
                    .unwrap();

                if trie.is_complete(&txn) {
                    break;
                }
                next_start = chunk.keys_end.unwrap();
            }
            assert!(trie.is_complete(&txn));
            assert_eq!(trie.count_nodes(&txn), (0, 2, 2));

            assert_eq!(trie.get(&txn, &key_1).expect("complete trie"), Some(80085));
            assert_eq!(trie.get(&txn, &key_2).expect("complete trie"), Some(999));
            assert_eq!(trie.get(&txn, &key_3).expect("complete trie"), Some(1337));
            assert_eq!(trie.get(&txn, &key_4).expect("complete trie"), Some(6969));
            assert_eq!(trie.get(&txn, &key_5).expect("complete trie"), None::<i32>);
        }
    }

    #[test]
    fn partial_tree_remove_chunks_of_different_sizes() {
        let key_1 = "413f22".parse().unwrap();
        let key_2 = "413".parse().unwrap();
        let key_3 = "413f227fa".parse().unwrap();
        let key_4 = "413b391".parse().unwrap();
        let key_5 = "412324".parse().unwrap();

        let env = nimiq_database::volatile::VolatileEnvironment::new(10).unwrap();
        let original = MerkleRadixTrie::new(env.clone(), "original");
        let tries: Vec<_> = (1..5)
            .map(|i| {
                let trie = MerkleRadixTrie::new(env.clone(), &format!("copy{}", i));
                let mut txn = WriteTransaction::new(&env);
                trie.put(&mut txn, &key_1, 80085).expect("complete trie");
                trie.put(&mut txn, &key_2, 999).expect("complete trie");
                trie.put(&mut txn, &key_3, 1337).expect("complete trie");
                trie.put(&mut txn, &key_4, 6969).expect("complete trie");
                trie.update_root(&mut txn);
                assert_eq!(trie.count_nodes(&txn), (0, 2, 2));
                assert!(trie.is_complete(&txn));
                txn.commit();

                (i, trie)
            })
            .collect();
        let mut txn = WriteTransaction::new(&env);

        original
            .put(&mut txn, &key_1, 80085)
            .expect("complete trie");
        original.put(&mut txn, &key_2, 999).expect("complete trie");
        original.put(&mut txn, &key_3, 1337).expect("complete trie");
        original.put(&mut txn, &key_4, 6969).expect("complete trie");
        original.update_root(&mut txn);
        assert_eq!(original.count_nodes(&txn), (0, 2, 2));
        assert!(original.is_complete(&txn));

        let node_keys: Vec<_> = original
            .get_trie_chunk(&txn, &KeyNibbles::ROOT, 4)
            .drain(..)
            .map(|node| node.key)
            .collect();

        for (chunk_size, trie) in tries {
            let mut next_start = node_keys[node_keys.len() - chunk_size].clone();
            for i in 0..10 {
                trie.remove_chunk(&mut txn, next_start.clone()).unwrap();
                next_start =
                    node_keys[node_keys.len().saturating_sub(chunk_size * (i + 1))].clone();

                if trie.num_leaves(&txn) == 0 && trie.num_hybrids(&txn) == 0 {
                    break;
                }
            }
            assert!(!trie.is_complete(&txn));
            assert_eq!(trie.count_nodes(&txn), (0, 0, 0));

            assert_eq!(
                trie.get::<TrieNode>(&txn, &key_1).err(),
                Some(IncompleteTrie)
            );
            assert_eq!(
                trie.get::<TrieNode>(&txn, &key_2).err(),
                Some(IncompleteTrie)
            );
            assert_eq!(
                trie.get::<TrieNode>(&txn, &key_3).err(),
                Some(IncompleteTrie)
            );
            assert_eq!(
                trie.get::<TrieNode>(&txn, &key_4).err(),
                Some(IncompleteTrie)
            );
            assert_eq!(
                trie.get::<i32>(&txn, &key_5).expect("Complete trie"),
                None::<i32>
            );
        }
    }

    #[test]
    #[ignore]
    fn trie_cache_consistency() {
        let key_1 = "413f22".parse().unwrap();
        let key_2 = "413".parse().unwrap();
        let key_3 = "413f227fa".parse().unwrap();
        let key_4 = "413b391".parse().unwrap();

        let env = nimiq_database::volatile::VolatileEnvironment::new(10).unwrap();
        let original = MerkleRadixTrie::new(env.clone(), "original");

        let mut txn = WriteTransaction::new(&env);

        original
            .put(&mut txn, &key_1, 80085)
            .expect("complete trie");
        original.put(&mut txn, &key_2, 999).expect("complete trie");
        original.put(&mut txn, &key_3, 1337).expect("complete trie");
        original.put(&mut txn, &key_4, 6969).expect("complete trie");
        original.update_root(&mut txn);
        assert_eq!(original.count_nodes(&txn), (0, 2, 2));
        assert!(original.is_complete(&txn));

        txn.abort();

        let txn = WriteTransaction::new(&env);
        assert_eq!(original.num_branches(&txn), 0);
        assert_eq!(original.num_hybrids(&txn), 0);
        assert_eq!(original.num_leaves(&txn), 0);
        assert_eq!(original.count_nodes(&txn), (0, 0, 0));
        assert!(original.is_complete(&txn));
    }

    #[test]
    fn can_handle_hybrid_node_with_one_child() {
        /*
        Will produce the following tree:

                        |
                       413
                       / \
                 ..b391   ..f22
                            |
                          ..7fa
         */
        let key_1 = "413f22".parse().unwrap();
        let key_2 = "413".parse().unwrap();
        let key_3 = "413f227fa".parse().unwrap();
        let key_4 = "413b391".parse().unwrap();

        let env = nimiq_database::volatile::VolatileEnvironment::new(10).unwrap();
        let original = MerkleRadixTrie::new(env.clone(), "original");

        let mut txn = WriteTransaction::new(&env);

        // Add the nodes and make sure put works correctly.
        original
            .put(&mut txn, &key_1, 80085) // 413f22
            .expect("complete trie");
        assert_eq!(original.count_nodes(&txn), (0, 0, 1));
        original.put(&mut txn, &key_2, 999).expect("complete trie"); // 413
        assert_eq!(original.count_nodes(&txn), (0, 1, 1));
        original.put(&mut txn, &key_3, 1337).expect("complete trie"); // 413f227fa
        assert_eq!(original.count_nodes(&txn), (0, 2, 1));
        original.put(&mut txn, &key_4, 6969).expect("complete trie"); // 413b391
        original.update_root(&mut txn);
        assert_eq!(original.count_nodes(&txn), (0, 2, 2));
        assert!(original.is_complete(&txn));

        // Remove the nodes and make sure remove works correctly.
        original.remove(&mut txn, &key_4); // 413b391
        assert_eq!(original.count_nodes(&txn), (0, 2, 1));
        original.remove(&mut txn, &key_3); // 413f227fa
        assert_eq!(original.count_nodes(&txn), (0, 1, 1));
        original.remove(&mut txn, &key_2); // 413
        assert_eq!(original.count_nodes(&txn), (0, 0, 1));
        original.remove(&mut txn, &key_1); // 413f22
        assert_eq!(original.count_nodes(&txn), (0, 0, 0));
    }

    #[test]
    fn remove_chunk_on_empty_tree() {
        let key_1 = "413f22".parse().unwrap();

        let env = nimiq_database::volatile::VolatileEnvironment::new(10).unwrap();
        let original = MerkleRadixTrie::new(env.clone(), "original");

        let mut txn = WriteTransaction::new(&env);

        original.remove_chunk(&mut txn, key_1).unwrap();
        assert!(original.is_complete(&txn));
    }

    #[test]
    fn remove_empty_chunk() {
        let key_1 = "413f22".parse().unwrap();
        let key_2 = "413".parse().unwrap();
        let key_3 = "413f227fa".parse().unwrap();
        let key_4 = "413b391".parse().unwrap();
        let key_5: KeyNibbles = "415324".parse().unwrap();

        let env = nimiq_database::volatile::VolatileEnvironment::new(10).unwrap();
        let original = MerkleRadixTrie::new(env.clone(), "original");

        let mut txn = WriteTransaction::new(&env);

        original
            .put(&mut txn, &key_1, 80085)
            .expect("complete trie");
        original.put(&mut txn, &key_2, 999).expect("complete trie");
        original.put(&mut txn, &key_3, 1337).expect("complete trie");
        original.put(&mut txn, &key_4, 6969).expect("complete trie");
        original.update_root(&mut txn);
        assert_eq!(original.count_nodes(&txn), (0, 2, 2));
        assert!(original.is_complete(&txn));

        original.remove_chunk(&mut txn, key_5.clone()).unwrap();
        assert_eq!(original.count_nodes(&txn), (0, 2, 2));
        assert!(original.is_complete(&txn));

        original.remove_chunk(&mut txn, key_3).unwrap();
        assert_eq!(original.count_nodes(&txn), (0, 1, 2));
        assert!(!original.is_complete(&txn));

        original.remove_chunk(&mut txn, key_5).unwrap();
        assert_eq!(original.count_nodes(&txn), (0, 1, 2));
        assert!(!original.is_complete(&txn));
    }

    #[test]
    fn remove_and_put_chunk() {
        let key_1 = "413f22".parse().unwrap();
        let key_2 = "413".parse().unwrap();
        let key_3 = "413f227fa".parse().unwrap();
        let key_4 = "413b391".parse().unwrap();

        let env = nimiq_database::volatile::VolatileEnvironment::new(10).unwrap();
        let original = MerkleRadixTrie::new(env.clone(), "original");

        let mut txn = WriteTransaction::new(&env);
        assert!(original.is_complete(&txn));

        original
            .put(&mut txn, &key_1, 80085)
            .expect("complete trie");
        original.put(&mut txn, &key_2, 999).expect("complete trie");
        original.put(&mut txn, &key_3, 1337).expect("complete trie");
        original.put(&mut txn, &key_4, 6969).expect("complete trie");
        original.update_root(&mut txn);
        assert_eq!(original.count_nodes(&txn), (0, 2, 2));
        assert!(original.is_complete(&txn));

        let hash = original.root_hash(&txn);
        let chunk = original.get_chunk_with_proof(&txn, key_1.clone().., 2);

        original.remove_chunk(&mut txn, key_1.clone()).unwrap();
        assert_eq!(original.count_nodes(&txn), (0, 1, 1));
        assert!(!original.is_complete(&txn));

        original
            .put_chunk(&mut txn, key_1, chunk.clone(), hash.clone())
            .unwrap();
        assert_eq!(original.count_nodes(&txn), (0, 2, 2));
        assert!(original.is_complete(&txn));
    }
}
