use std::{
    cmp,
    collections::{BTreeMap, BTreeSet},
    marker::PhantomData,
    mem, ops,
};

use log::error;
use nimiq_database::{
    traits::{Database, ReadCursor, ReadTransaction, WriteTransaction},
    DatabaseProxy, IntoIterProxy, TableProxy, TransactionProxy,
};
use nimiq_hash::Blake2bHash;
use nimiq_primitives::{
    key_nibbles::KeyNibbles,
    trie::{
        error::{IncompleteTrie, MerkleRadixTrieError},
        trie_chunk::{TrieChunk, TrieChunkPushResult, TrieItem},
        trie_diff::{RevertDiffValue, RevertTrieDiff, TrieDiff},
        trie_node::{RootData, TrieNode, TrieNodeKind},
        trie_proof::TrieProof,
        trie_proof_node::TrieProofNode,
    },
};
use nimiq_serde::{Deserialize, Serialize};

use crate::{
    transaction::{OldValue, TransactionExt as _},
    WriteTransactionProxy,
};

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
    db: TableProxy,
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

impl MerkleRadixTrie {
    /// Start a new Merkle Radix Trie with the given Environment and the given name.
    pub fn new(db: DatabaseProxy, name: &str) -> Self {
        Self::new_impl(db, name, false)
    }

    pub fn new_incomplete(env: DatabaseProxy, name: &str) -> Self {
        Self::new_impl(env, name, true)
    }

    fn new_impl(db: DatabaseProxy, name: &str, incomplete: bool) -> Self {
        let table = db.open_table(name.to_string());

        let tree = MerkleRadixTrie { db: table };

        let mut txn = db.write_transaction();
        tree.init_root(&mut (&mut txn).into(), incomplete);
        txn.commit();

        tree
    }

    fn init_root(&self, txn: &mut WriteTransactionProxy, incomplete: bool) {
        if self.get_root(txn).is_none() {
            let root = if incomplete {
                TrieNode::new_root_incomplete()
            } else {
                TrieNode::new_root()
            };
            self.put_node(txn, &root, OldValue::None);
        }
    }

    pub fn init(&self, txn: &mut WriteTransactionProxy, values: Vec<TrieItem>) {
        assert!(self.is_complete(txn));
        assert_eq!(self.num_leaves(txn), 0);
        assert_eq!(self.num_hybrids(txn), 0);

        for item in values {
            self.put_raw(txn, &item.key, item.value, &None);
        }

        self.update_root(txn).expect("Tree must be complete");
    }

    /// Clears the database and initializes it as incomplete.
    pub fn reinitialize_as_incomplete(&self, txn: &mut WriteTransactionProxy) {
        txn.clear_database(&self.db);
        self.init_root(txn, true);
    }

    /// Prints a human friendly version of the subtrie for debugging.
    /// Not to be used on large trees!
    pub fn debug_print(&self, txn: &TransactionProxy) {
        let missing_range = self.get_missing_range(txn);
        println!("-> ROOT");
        self.debug_print_subtrie(txn, &KeyNibbles::ROOT, &missing_range, 2)
    }

    /// Prints a human friendly version of the subtrie for debugging.
    /// Not to be used on large trees!
    pub fn debug_print_subtrie(
        &self,
        txn: &TransactionProxy,
        key: &KeyNibbles,
        missing_range: &Option<ops::RangeFrom<KeyNibbles>>,
        depth: usize,
    ) {
        let prefix = "  ".repeat(depth);
        let node = if let Some(node) = self.get_node(txn, key) {
            node
        } else {
            println!("{}-> MISSING", prefix);
            return;
        };

        for child in node.iter_children() {
            println!("{}-> {} ({})", prefix, child.suffix, child.has_hash());
            if let Ok(subkey) = child.key(key, missing_range) {
                self.debug_print_subtrie(txn, &subkey, missing_range, depth + 2);
            } else {
                println!("{}  -> MISSING", prefix);
            }
        }
    }

    /// Returns the root hash of the Merkle Radix Trie.
    pub fn root_hash_assert(&self, txn: &TransactionProxy) -> Blake2bHash {
        let root = self.get_root(txn).unwrap();

        // We should refuse to return a hash for a completely empty but incomplete trie.
        assert!(
            self.is_complete(txn) || root.has_children(),
            "Cannot compute hash on an empty, incomplete trie"
        );

        root.hash_assert()
    }

    /// Returns the root hash of the Merkle Radix Trie if the trie is complete.
    pub fn root_hash(&self, txn: &TransactionProxy) -> Option<Blake2bHash> {
        let root = self.get_root(txn).unwrap();

        // We should refuse to return a hash for a completely empty but incomplete trie.
        if !self.is_complete(txn) && !root.has_children() {
            return None;
        }

        let root_hash = root.hash();

        // Make sure the root hash can only be None if the trie is incomplete.
        assert!(root_hash.is_some() || !self.is_complete(txn));

        root_hash
    }

    pub fn is_complete(&self, txn: &TransactionProxy) -> bool {
        self.get_root(txn)
            .unwrap()
            .root_data
            .unwrap()
            .incomplete
            .is_none()
    }

    /// Returns the number of branch nodes in the Merkle Radix Trie.
    pub fn num_branches(&self, txn: &TransactionProxy) -> u64 {
        self.get_root(txn).unwrap().root_data.unwrap().num_branches
    }

    /// Returns the number of hybrid nodes in the Merkle Radix Trie.
    pub fn num_hybrids(&self, txn: &TransactionProxy) -> u64 {
        self.get_root(txn).unwrap().root_data.unwrap().num_hybrids
    }

    /// Returns the number of leaf nodes in the Merkle Radix Trie.
    pub fn num_leaves(&self, txn: &TransactionProxy) -> u64 {
        self.get_root(txn).unwrap().root_data.unwrap().num_leaves
    }

    #[cfg(test)]
    fn count_nodes(&self, txn: &TransactionProxy) -> (u64, u64, u64) {
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

    fn get_node(&self, txn: &TransactionProxy, key: &KeyNibbles) -> Option<TrieNode> {
        txn.get_node(&self.db, key)
    }

    fn put_node(&self, txn: &mut WriteTransactionProxy, node: &TrieNode, old_value: OldValue) {
        txn.put_node(&self.db, node, old_value)
    }

    fn remove_node(&self, txn: &mut WriteTransactionProxy, key: &KeyNibbles, old_value: OldValue) {
        txn.remove_node(&self.db, key, old_value)
    }

    /// Get the value at the given key. If there's no leaf or hybrid node at the given key then it
    /// returns None.
    pub fn get<T: Deserialize>(
        &self,
        txn: &TransactionProxy,
        key: &KeyNibbles,
    ) -> Result<Option<T>, IncompleteTrie> {
        let missing_range = self.get_missing_range(txn);
        if !self.is_within_complete_part(key, &missing_range) {
            return Err(IncompleteTrie);
        }
        Ok(self
            .get_raw(txn, key)
            .map(|v| T::deserialize_from_vec(&v).unwrap()))
    }

    fn get_raw(&self, txn: &TransactionProxy, key: &KeyNibbles) -> Option<Vec<u8>> {
        self.get_node(txn, key)?.value
    }

    /// Insert a value into the Merkle Radix Trie at the given key. If the key already exists then
    /// it will overwrite it. You can't use this function to check the existence of a given key.
    pub fn put<T: Serialize>(
        &self,
        txn: &mut WriteTransactionProxy,
        key: &KeyNibbles,
        value: T,
    ) -> Result<(), MerkleRadixTrieError> {
        // PITODO: Return value needs to change, we don't need the error anymore
        let missing_range = self.get_missing_range(txn);
        if self.is_within_complete_part(key, &missing_range) {
            self.put_raw(txn, key, value.serialize_to_vec(), &missing_range);
        } else {
            self.update_within_missing_part_raw(txn, key, &missing_range)?;
        }

        Ok(())
    }

    /// Removes the value in the Merkle Radix Trie at the given key. If the key doesn't exist
    /// then this function just returns silently. You can't use this to check the existence of a
    /// given prefix.
    pub fn remove(&self, txn: &mut WriteTransactionProxy, key: &KeyNibbles) {
        let missing_range = self.get_missing_range(txn);
        if self.is_within_complete_part(key, &missing_range) {
            self.remove_raw(txn, key, &missing_range);
        } else {
            // PITODO: return error
            self.update_within_missing_part_raw(txn, key, &missing_range)
                .expect("should not happen or be returned");
        }
    }

    /// Insert a value into the Merkle Radix Trie at the given key. If the key already exists then
    /// it will overwrite it. You can't use this function to check the existence of a given key.
    fn put_raw(
        &self,
        txn: &mut WriteTransactionProxy,
        key: &KeyNibbles,
        value: Vec<u8>,
        missing_range: &Option<ops::RangeFrom<KeyNibbles>>,
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
                    self.put_node(txn, &new_node, OldValue::None);

                    // Push the new node into the root path.
                    root_path.push(new_node);
                    count_updates = CountUpdates {
                        hybrids: 1,
                        ..Default::default()
                    };
                } else {
                    // The new node is a sibling of the current node. Thus it is a leaf node.
                    let new_node = TrieNode::new_leaf(key.clone(), value);
                    self.put_node(txn, &new_node, OldValue::None);

                    // We insert a new branch node as the parent of both the current node and the
                    // new node.
                    let mut new_parent = TrieNode::new_empty(cur_node.key.common_prefix(key));
                    new_parent.put_child_no_hash(&cur_node.key).unwrap();
                    new_parent
                        .put_child(&new_node.key, new_node.hash_assert())
                        .unwrap();
                    self.put_node(txn, &new_parent, OldValue::None);

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
                let old_value = cur_node.put_value(value).unwrap();
                self.put_node(txn, &cur_node, old_value.into());

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
                    self.put_node(txn, &new_node, OldValue::None);

                    // Update the parent node and store it.
                    let old_kind = cur_node.kind();
                    cur_node
                        .put_child(&new_node.key, new_node.hash_assert())
                        .unwrap();
                    self.put_node(txn, &cur_node, OldValue::Unchanged);

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

                    cur_node = txn.get_node(&self.db, &child_key).unwrap();
                }
            }
        }
        // Update the keys and hashes of the nodes that we modified.
        self.update_keys(txn, root_path, count_updates);
    }

    pub fn update_within_missing_part(
        &self,
        txn: &mut WriteTransactionProxy,
        key: &KeyNibbles,
    ) -> Result<(), MerkleRadixTrieError> {
        let missing_range = self.get_missing_range(txn);
        self.update_within_missing_part_raw(txn, key, &missing_range)?;
        Ok(())
    }

    /// Resets the hashes for a path to a node in the missing part of the tree.
    /// This is used to indicate that the corresponding branch's hash has changed,
    /// but we cannot provide the accurate hash until a new chunk has been pushed.
    ///
    /// Returns `true` if a new stump has been added, otherwise `false`.
    fn update_within_missing_part_raw(
        &self,
        txn: &mut WriteTransactionProxy,
        key: &KeyNibbles,
        missing_range: &Option<ops::RangeFrom<KeyNibbles>>,
    ) -> Result<bool, MerkleRadixTrieError> {
        let mut node = self
            .get_root(txn)
            .expect("The Merkle Radix Trie didn't have a root node!");

        let mut root_path = Vec::new();
        let mut new_stump = false;
        // Descend down the tree and collect nodes to be updated.
        loop {
            // This function should only be called with keys in the missing range.
            assert_ne!(&node.key, key);

            // Check that the key we are trying to update can exist in this part of the tree.
            if !node.key.is_prefix_of(key) {
                return Err(MerkleRadixTrieError::ChildDoesNotExist);
            }

            // Descend to the corresponding child.
            match node.child_key(key, missing_range) {
                Ok(child_key) => {
                    let child = self
                        .get_node(txn, &child_key)
                        .expect("Child should be present");
                    let parent = mem::replace(&mut node, child);
                    root_path.push(parent);
                }
                e @ Err(MerkleRadixTrieError::ChildIsStump)
                | e @ Err(MerkleRadixTrieError::ChildDoesNotExist) => {
                    // We cannot continue further down as the next node is outside our trie.
                    // Update this node prior to `update_keys` (which only updates the other nodes in the root path).
                    node.put_child_no_hash(key).expect("Prefix must be correct");
                    self.put_node(txn, &node, OldValue::Unchanged);

                    // Mark that a new stump has been added.
                    if let Err(MerkleRadixTrieError::ChildDoesNotExist) = e {
                        new_stump = true;
                    }

                    root_path.push(node);
                    break;
                }
                Err(e) => {
                    return Err(e);
                }
            }
        }

        self.update_keys(txn, root_path, CountUpdates::default());

        Ok(new_stump)
    }

    /// Removes the corresponding stump and updates the hashes for a path
    /// to a node in the missing part of the tree.
    /// This is used to indicate that the corresponding branch's hash has changed,
    /// but we cannot provide the accurate hash until a new chunk has been pushed.
    fn remove_within_missing_part_raw(
        &self,
        txn: &mut WriteTransactionProxy,
        key: &KeyNibbles,
        missing_range: &Option<ops::RangeFrom<KeyNibbles>>,
    ) -> Result<(), MerkleRadixTrieError> {
        let mut node = self
            .get_root(txn)
            .expect("The Merkle Radix Trie didn't have a root node!");

        let mut root_path = Vec::new();
        // Descend down the tree and collect nodes to be updated.
        loop {
            // This function should only be called with keys in the missing range.
            assert_ne!(&node.key, key);

            // Check that the key we are trying to update can exist in this part of the tree.
            if !node.key.is_prefix_of(key) {
                return Err(MerkleRadixTrieError::ChildDoesNotExist);
            }

            // Descend to the corresponding child.
            match node.child_key(key, missing_range) {
                Ok(child_key) => {
                    let child = self
                        .get_node(txn, &child_key)
                        .expect("Child should be present");
                    let parent = mem::replace(&mut node, child);
                    root_path.push(parent);
                }
                Err(MerkleRadixTrieError::ChildIsStump) => {
                    // We cannot continue further down as the next node is outside our trie.
                    // Remove this stump.
                    node.remove_child(key).expect("Prefix must be correct");
                    self.put_node(txn, &node, OldValue::Unchanged);

                    root_path.push(node);
                    break;
                }
                Err(MerkleRadixTrieError::ChildDoesNotExist) => unreachable!("Child must exist"),
                Err(e) => {
                    return Err(e);
                }
            }
        }

        self.update_keys(txn, root_path, CountUpdates::default());

        Ok(())
    }

    pub fn get_chunk_with_proof(
        &self,
        txn: &TransactionProxy,
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
            .map(|n| TrieItem::new(n.key, n.value.unwrap()))
            .collect();
        let proof = if let Some(item) = chunk.last() {
            // Get the proof for the last included item if available.
            self.get_proof(txn, vec![&item.key])
                .expect("trie proof failed for last item")
        } else if let Some(pred) = self.get_predecessor(txn, &keys.start) {
            // Else for the last item already included in the trie.
            self.get_proof(txn, vec![&pred])
                .expect("trie proof failed for predecessor")
        } else {
            // Else just for the root if there are no items in the trie.
            TrieProof::new(vec![self.get_root(txn).unwrap().into()], Default::default())
        };
        // The end can contain one nibble beyond the shared prefix of the next item not included and
        // the last item still included in the chunk.
        let end = first_not_included.map(|key| {
            let max_len = proof.nodes.first().unwrap().key.common_prefix(&key).len() + 1;
            key.slice(0, cmp::min(key.len(), max_len))
        });
        TrieChunk {
            end_key: end,
            items: chunk,
            proof,
        }
    }

    /// Removes empty stump children on the rightmost path in the tree.
    /// These stumps are stored in the existing nodes to mark the (yet) missing parts of the partial tree.
    fn clear_stumps(&self, txn: &mut WriteTransactionProxy, keys: ops::RangeFrom<KeyNibbles>) {
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
                    + usize::from(!is_last_digit);
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
                    self.remove_node(txn, &cur_node.key, OldValue::Unchanged);

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
                }
                self.put_node(txn, &cur_node, OldValue::Unchanged);
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
        txn: &mut WriteTransactionProxy,
        keys: ops::RangeFrom<KeyNibbles>,
        last_item_proof: &[TrieProofNode],
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

            self.put_node(txn, &cur_node, OldValue::Unchanged);

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

    /// When pushing a chunk the correct behavior may result in an applied chunk or in an ignored chunk.
    /// `start_key` is inclusive and is meant to check if the chunk is a consecutive chunk.
    pub fn put_chunk(
        &self,
        txn: &mut WriteTransactionProxy,
        start_key: KeyNibbles,
        chunk: TrieChunk,
        expected_hash: Blake2bHash,
    ) -> Result<TrieChunkPushResult, MerkleRadixTrieError> {
        match self.get_root(txn).unwrap().root_data.unwrap().incomplete {
            Some(i) if i.start == start_key => {}
            Some(_) => return Ok(TrieChunkPushResult::Ignored),
            None => return Err(MerkleRadixTrieError::TrieAlreadyComplete),
        }

        if let Some(end_key) = &chunk.end_key {
            if start_key > *end_key {
                return Err(MerkleRadixTrieError::InvalidChunk("invalid keys"));
            }
        }
        if !chunk.items.is_empty() {
            if start_key > chunk.items[0].key {
                return Err(MerkleRadixTrieError::InvalidChunk(
                    "first key is inconsistent with key range",
                ));
            }
            let last_item_key = chunk.items.last().unwrap().key.clone();
            if let Some(end_key) = &chunk.end_key {
                if last_item_key >= *end_key {
                    return Err(MerkleRadixTrieError::InvalidChunk(
                        "last key is inconsistent with key range",
                    ));
                }
            }
            let end_key_range = chunk.end_key.clone().map(|end_key| end_key..);
            for proof_node in chunk.proof.nodes.iter() {
                for child in proof_node.iter_children() {
                    if let Ok(key) = child.key(&proof_node.key, &end_key_range) {
                        if key > last_item_key {
                            return Err(MerkleRadixTrieError::InvalidChunk(
                                "invalid keys end, found stumps",
                            ));
                        }
                    }
                }
            }

            for i in 0..chunk.items.len() - 1 {
                if chunk.items[i].key >= chunk.items[i + 1].key {
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
        if let Some(end_key) = &chunk.end_key {
            let mut is_valid_end = false;
            if let Some(last_proof_node) = chunk.proof.nodes.first() {
                if last_proof_node.key == *end_key {
                    is_valid_end = true;
                }
            }
            if !is_valid_end {
                for proof_node in &chunk.proof.nodes {
                    if proof_node.key.len() + 1 == end_key.len()
                        && proof_node.key.is_prefix_of(end_key)
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
                if last_proof_node.key >= *end_key {
                    return Err(MerkleRadixTrieError::InvalidChunk(
                        "end key inconsistent with last proof node",
                    ));
                }
            }
        }

        // First, clear the tree's stumps.
        self.clear_stumps(txn, start_key..);
        // Then, put all the new items.
        let missing_range = chunk.end_key.clone().map(|end| end..);
        for item in chunk.items {
            self.put_raw(txn, &item.key, item.value, &missing_range);
        }

        // Mark the remaining stumps.
        if let Some(end_key) = &chunk.end_key {
            self.mark_stumps(txn, end_key.clone().., &chunk.proof.nodes)?;
        }

        // Update the hashes and check that we're good.
        let actual_hash = self
            .update_hashes(txn, &KeyNibbles::ROOT, &missing_range)
            .map_err(|_| MerkleRadixTrieError::ChunkHashMismatch)?;

        if actual_hash != expected_hash {
            error!(
                "Putting chunk failed, have hash {} wanted hash {}",
                actual_hash, expected_hash
            );
            return Err(MerkleRadixTrieError::ChunkHashMismatch);
        }

        let mut root_node = self.get_root(txn).unwrap();
        root_node.root_data.as_mut().unwrap().incomplete = chunk.end_key.clone().map(|end| end..);
        self.put_node(txn, &root_node, OldValue::Unchanged);

        Ok(TrieChunkPushResult::Applied)
    }

    /// `start_key` is inclusive and marks the first key to be removed.
    pub fn remove_chunk(
        &self,
        txn: &mut WriteTransactionProxy,
        start_key: KeyNibbles,
    ) -> Result<(), MerkleRadixTrieError> {
        let missing_range = self.get_missing_range(txn);

        // PITODO: Optimize this.
        // Currently, we need the whole list of remaining nodes to generate the chunk proof,
        // and the remaining list of nodes to remove those.
        let to_remove: Vec<_> = self
            .get_trie_chunk(
                txn,
                &start_key,
                (self.num_leaves(txn) + self.num_hybrids(txn)) as usize, //PITODO: optimize this
            )
            .drain(..)
            .map(|node| node.key)
            .collect();

        if to_remove.is_empty() {
            return Ok(());
        }

        // Next, we get a proof for the last remaining leaf.
        let proof = if let Some(last_remaining) = self.get_predecessor(txn, &start_key) {
            self.get_proof(txn, vec![&last_remaining]).unwrap()
        } else {
            TrieProof::new(vec![self.get_root(txn).unwrap().into()], Default::default())
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
        self.mark_stumps(txn, start_key.clone().., &proof.nodes)?;

        let mut root_node = self.get_root(txn).unwrap();
        root_node.root_data.as_mut().unwrap().incomplete =
            Some(ops::RangeFrom { start: start_key });
        self.put_node(txn, &root_node, OldValue::Unchanged);

        Ok(())
    }

    /// Removes the value in the Merkle Radix Trie at the given key. If the key doesn't exist
    /// then this function just returns silently. You can't use this to check the existence of a
    /// given prefix.
    fn remove_raw(
        &self,
        txn: &mut WriteTransactionProxy,
        key: &KeyNibbles,
        missing_range: &Option<ops::RangeFrom<KeyNibbles>>,
    ) {
        // PITODO: add result type
        // Start by getting the root node.
        let mut cur_node = self
            .get_root(txn)
            .expect("Patricia Trie must have a root node!");

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
                let prev_kind = cur_node.kind();
                let old_value = cur_node.value.take();
                let count_updates;
                if cur_node.is_root() || cur_node.has_children() {
                    // Node was a hybrid node and is now a branch node.
                    let num_children = cur_node.iter_children().count();

                    // If it has only a single child and isn't the root node, merge it with that child.
                    if num_children == 1 && !cur_node.is_root() {
                        // Remove the node from the database.
                        self.remove_node(txn, &cur_node.key, old_value.into());

                        // Get the node's only child and add it to the root path.
                        let only_child_key = cur_node
                            .iter_children()
                            .next()
                            .unwrap()
                            .key(&cur_node.key, missing_range)
                            .expect("no stump");

                        let only_child = self.get_node(txn, &only_child_key).unwrap();

                        root_path.push(only_child);

                        // We removed a hybrid node.
                        count_updates = CountUpdates {
                            hybrids: -1,
                            ..Default::default()
                        };
                    } else {
                        // The node is root or has multiple children, thus we cannot remove it.
                        // Instead we converted it to a branch node by removing its value.

                        // We converted a hybrid node into a branch node.
                        // Or kept a branch node a branch node if there was no value stored before.
                        count_updates = CountUpdates::from_update(prev_kind, cur_node.kind());

                        // Update the node and add it to the root path.
                        self.put_node(txn, &cur_node, old_value.into());

                        root_path.push(cur_node);
                    };

                    // Update the keys and hashes of the rest of the root path.
                    self.update_keys(txn, root_path, count_updates);

                    return;
                } else {
                    // Node was a leaf node, delete if from the database.
                    self.remove_node(txn, key, old_value.into());
                    break;
                }
            }

            // Try to find a child of the current node that matches our key.
            match cur_node.child_key(key, missing_range) {
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
                self.remove_node(txn, &parent_node.key, OldValue::Unchanged);
                count_updates.apply_update(parent_node.kind(), None);

                // Get the node's only child and add it to the root path.
                let only_child_key = parent_node
                    .iter_children()
                    .next()
                    .unwrap()
                    .key(&parent_node.key, missing_range)
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
                self.put_node(txn, &parent_node, OldValue::Unchanged);

                root_path.push(parent_node);

                self.update_keys(txn, root_path, count_updates);

                return;
            }
            // Otherwise, our node must have no children and not be the root/hybrid node. In this case we
            // need to remove it too, so we loop again.
            else {
                self.remove_node(txn, &parent_node.key, OldValue::Unchanged);
                count_updates.apply_update(parent_node.kind(), None);
                child_key = parent_node.key.clone();
            }
        }
    }

    /// Produces a Merkle proof of the inclusion of the given keys in the
    /// Merkle Radix Trie.
    ///
    /// The proof consists of the path from the leaves that
    /// we want to prove inclusion all the way up to the root. For example, for
    /// the following trie:
    ///
    /// ```text
    ///              R
    ///              |
    ///              B1
    ///          /   |   \
    ///        B2   L3   B3
    ///       / \        / \
    ///      L1 L2      L4 L5
    /// ```
    ///
    /// If we want a proof for the nodes L1 and L3, the proof will consist of
    /// the nodes L1, B2, L3, B1 and R.
    ///
    /// Note that:
    ///
    /// 1. Unlike Merkle proofs we don't need the adjacent branch nodes. That's
    ///    because our branch nodes already include the hashes of its children.
    ///
    /// 2. The nodes are always returned in post-order.
    pub fn get_proof(
        &self,
        txn: &TransactionProxy,
        mut keys: Vec<&KeyNibbles>,
    ) -> Result<TrieProof, IncompleteTrie> {
        // We sort the keys in post-order.
        keys.sort_by(|&k1, &k2| k1.post_order_cmp(k2));

        let missing_range = self.get_missing_range(txn);
        if let Some(missing) = &missing_range {
            if keys.iter().any(|&key| missing.contains(key)) {
                return Err(IncompleteTrie);
            }
        }

        let wanted_values: BTreeSet<&KeyNibbles> = keys.iter().cloned().collect();
        let to_proof = |node: TrieNode| TrieProofNode::new(wanted_values.contains(&node.key), node);

        // Initialize the vector that will contain the proof.
        let mut proof_nodes = Vec::new();

        // Initialize the map that specifies which keys are only proven indirectly.
        let mut missing_proven_by = BTreeMap::new();

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

        // Iterate over all the keys that we wish to prove.
        loop {
            // Go down the trie until we find a node with our key or we can't go any further.
            loop {
                // If the key fully matches, we have found the requested node.
                if pointer_node.key == *cur_key {
                    break;
                }

                // Otherwise, try to find a child of the pointer node that matches our key.
                match pointer_node.child_key(cur_key, &missing_range) {
                    // If there's a child, check whether it's on the right path.
                    //
                    // If so, we update the pointer node and the root path, and
                    // continue down the trie.
                    Ok(child_key) if child_key.is_prefix_of(cur_key) => {
                        let old_pointer_node = mem::replace(
                            &mut pointer_node,
                            self.get_node(txn, &child_key).unwrap(),
                        );
                        root_path.push(old_pointer_node);
                    }
                    // Otherwise, no matching child exists, so the requested key is not part of this
                    // trie.
                    _ => {
                        assert!(missing_proven_by
                            .insert(cur_key.clone(), pointer_node.key.clone())
                            .is_none());
                        break;
                    }
                }
            }

            // Get the next key. If there's no next key then we get out of the loop.
            match keys.pop() {
                None => {
                    // Add the remaining nodes in the root path to the proof. Evidently they must
                    // be added in the reverse order.
                    proof_nodes.push(to_proof(pointer_node));
                    root_path.reverse();
                    proof_nodes.extend(root_path.drain(..).map(to_proof));

                    // Exit the loop.
                    break;
                }
                Some(key) => cur_key = key,
            }

            // Go up the root path until we get to a node that is a prefix to our current key.
            // Add the nodes you remove to the proof.
            while !pointer_node.key.is_prefix_of(cur_key) {
                let old_pointer_node = mem::replace(
                    &mut pointer_node,
                    root_path
                        .pop()
                        .expect("Root path must contain at least the root node!"),
                );

                proof_nodes.push(to_proof(old_pointer_node));
            }
        }

        proof_nodes.sort_by(|n1, n2| n1.key.post_order_cmp(&n2.key));

        // Return the proof.
        Ok(TrieProof::new(proof_nodes, missing_proven_by))
    }

    pub fn update_root(&self, txn: &mut WriteTransactionProxy) -> Result<(), MerkleRadixTrieError> {
        let missing_range = self.get_missing_range(txn);
        self.update_hashes(txn, &KeyNibbles::ROOT, &missing_range)
            .map_err(|_| MerkleRadixTrieError::IncompleteTrie)?;
        Ok(())
    }

    pub fn apply_diff(
        &self,
        txn: &mut WriteTransactionProxy,
        diff: TrieDiff,
    ) -> Result<RevertTrieDiff, MerkleRadixTrieError> {
        let missing_range = self.get_missing_range(txn);
        let mut result = BTreeMap::default();
        for (key, value) in diff.0 {
            if self.is_within_complete_part(&key, &missing_range) {
                let old_value = self.get_raw(txn, &key);
                if let Some(value) = value {
                    self.put_raw(txn, &key, value, &missing_range);
                } else {
                    self.remove_raw(txn, &key, &missing_range);
                };
                assert!(result
                    .insert(key, RevertDiffValue::known_value(old_value))
                    .is_none());
            } else {
                let new_stump = self.update_within_missing_part_raw(txn, &key, &missing_range)?;
                assert!(result
                    .insert(key, RevertDiffValue::unknown_value(new_stump))
                    .is_none());
            }
        }
        Ok(RevertTrieDiff(result))
    }

    pub fn revert_diff(
        &self,
        txn: &mut WriteTransactionProxy,
        diff: RevertTrieDiff,
    ) -> Result<(), MerkleRadixTrieError> {
        let missing_range = self.get_missing_range(txn);
        for (key, value) in diff.0.into_iter().rev() {
            if self.is_within_complete_part(&key, &missing_range) {
                match value {
                    RevertDiffValue::Put(value) => self.put_raw(txn, &key, value, &missing_range),
                    RevertDiffValue::Remove => self.remove_raw(txn, &key, &missing_range),
                    RevertDiffValue::UpdateStump => {
                        unreachable!("key should be in complete part")
                    }
                }
            } else {
                match value {
                    RevertDiffValue::Put(_) => unreachable!("key should be in incomplete part"),
                    RevertDiffValue::Remove => {
                        self.remove_within_missing_part_raw(txn, &key, &missing_range)?
                    }
                    RevertDiffValue::UpdateStump => {
                        self.update_within_missing_part_raw(txn, &key, &missing_range)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn is_within_complete_part(
        &self,
        key: &KeyNibbles,
        missing_range: &Option<ops::RangeFrom<KeyNibbles>>,
    ) -> bool {
        missing_range
            .as_ref()
            .map(|r| !r.contains(key))
            .unwrap_or(true)
    }

    /// Returns the range of missing keys in the partial tree.
    /// If the tree is complete, it returns `None`.
    pub fn get_missing_range(&self, txn: &TransactionProxy) -> Option<ops::RangeFrom<KeyNibbles>> {
        self.get_root(txn)
            .expect("trie needs root node")
            .root_data
            .expect("root node needs root data")
            .incomplete
    }

    /// Returns the root node, if there is one.
    fn get_root(&self, txn: &TransactionProxy) -> Option<TrieNode> {
        self.get_node(txn, &KeyNibbles::ROOT)
    }

    /// Updates the keys for a chain of nodes and marks those nodes as dirty. It assumes that the
    /// path starts at the root node and that each consecutive node is a child of the previous node.
    fn update_keys(
        &self,
        txn: &mut WriteTransactionProxy,
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
                    self.put_node(txn, root, OldValue::Unchanged);
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
            assert_eq!(root_path.is_empty(), parent_node.is_root());
            self.put_node(txn, &parent_node, OldValue::Unchanged);

            child_node = parent_node;
        }
    }

    /// Updates the hashes of all dirty nodes in the subtree specified by `key`.
    fn update_hashes(
        &self,
        txn: &mut WriteTransactionProxy,
        key: &KeyNibbles,
        missing_range: &Option<ops::RangeFrom<KeyNibbles>>,
    ) -> Result<Blake2bHash, IncompleteTrie> {
        let mut node: TrieNode = self.get_node(txn, key).unwrap();
        if !node.has_children() {
            return node.hash().ok_or(IncompleteTrie);
        }

        // Compute sub hashes if necessary.
        // Update everything that is possible to be updated and only return the potential error afterwards.
        for child in node.iter_children_mut() {
            if !child.has_hash() {
                // We only update the hashes for non-stump children.
                let child_key = match child.key(key, missing_range) {
                    Ok(key) => key,
                    Err(MerkleRadixTrieError::ChildIsStump) => continue,
                    _ => unreachable!(),
                };
                // TODO This could be parallelized.
                if let Ok(hash) = self.update_hashes(txn, &child_key, missing_range) {
                    child.hash = hash;
                }
            }
        }
        self.put_node(txn, &node, OldValue::Unchanged);
        node.hash().ok_or(IncompleteTrie)
    }

    /// Returns the last key containing a value before the given key.
    fn get_predecessor(&self, txn: &TransactionProxy, key: &KeyNibbles) -> Option<KeyNibbles> {
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
    fn get_trie_chunk(
        &self,
        txn: &TransactionProxy,
        start: &KeyNibbles,
        size: usize,
    ) -> Vec<TrieNode> {
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

    pub fn iter_nodes<'txn, T: Deserialize>(
        &self,
        txn: &'txn TransactionProxy,
        start_key: &KeyNibbles,
        end_key: &KeyNibbles,
    ) -> TrieNodeIter<'txn, T> {
        assert_eq!(
            start_key.len(),
            end_key.len(),
            "Start and end keys should have the same length"
        );
        TrieNodeIter::new(&self.db, txn, start_key, end_key.clone())
    }
}

/// This iterator is meant to start at `start_key` and finish at `end_key`, both of these are inclusive.
pub struct TrieNodeIter<'txn, T> {
    iter: IntoIterProxy<'txn, KeyNibbles, TrieNode>,
    end_key: KeyNibbles,
    _type: PhantomData<T>,
}

impl<'txn, T> TrieNodeIter<'txn, T> {
    /// This iterator is meant to start at `start_key` and finish at `end_key`, both of these are inclusive.
    fn new(
        db: &TableProxy,
        txn: &TransactionProxy,
        start_key: &KeyNibbles,
        end_key: KeyNibbles,
    ) -> Self {
        let cursor = txn.cursor(db);

        Self {
            iter: cursor.into_iter_from(start_key),
            end_key,
            _type: PhantomData,
        }
    }
}

impl<'txn, T: Deserialize> Iterator for TrieNodeIter<'txn, T> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        let (k, v) = self.iter.next()?;

        if k <= self.end_key {
            return T::deserialize_from_vec(&v.value?).ok();
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use nimiq_primitives::trie::trie_diff::{TrieDiffBuilder, ValueChange};
    use nimiq_test_log::test;

    use super::*;

    #[test]
    fn get_put_remove_works() {
        let key_1 = "413f22b3e".parse().unwrap();
        let key_2 = "413b39931".parse().unwrap();
        let key_3 = "413b397fa".parse().unwrap();
        let key_4 = "cfb986f5a".parse().unwrap();

        let env = nimiq_database::volatile::VolatileDatabase::new(20).unwrap();
        let trie = MerkleRadixTrie::new(env.clone(), "database");
        let mut raw_txn = env.write_transaction();
        let mut txn: WriteTransactionProxy = (&mut raw_txn).into();

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

        trie.remove(&mut txn, &KeyNibbles::ROOT);
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

        let env = nimiq_database::volatile::VolatileDatabase::new(20).unwrap();
        let trie = MerkleRadixTrie::new(env.clone(), "database");
        let mut raw_txn = env.write_transaction();
        let mut txn: WriteTransactionProxy = (&mut raw_txn).into();

        trie.put(&mut txn, &key_1, 9).expect("complete trie");
        trie.put(&mut txn, &key_2, 8).expect("complete trie");
        trie.put(&mut txn, &key_3, 7).expect("complete trie");
        trie.update_root(&mut txn).expect("complete trie");

        let proof = trie.get_proof(&txn, vec![&key_1, &key_2, &key_3]).unwrap();
        assert_eq!(proof.nodes.len(), 6);
        assert!(proof.verify(&trie.root_hash_assert(&txn)));

        let proof = trie.get_proof(&txn, vec![&key_3, &key_1, &key_2]).unwrap();
        assert_eq!(proof.nodes.len(), 6);
        assert!(proof.verify(&trie.root_hash_assert(&txn)));

        let proof = trie.get_proof(&txn, vec![&key_1, &key_3]).unwrap();
        assert_eq!(proof.nodes.len(), 5);
        assert!(proof.verify(&trie.root_hash_assert(&txn)));

        let proof = trie.get_proof(&txn, vec![&key_1, &key_2]).unwrap();
        assert_eq!(proof.nodes.len(), 5);
        assert!(proof.verify(&trie.root_hash_assert(&txn)));

        let proof = trie.get_proof(&txn, vec![&key_1]).unwrap();
        assert_eq!(proof.nodes.len(), 4);
        assert!(proof.verify(&trie.root_hash_assert(&txn)));

        let proof = trie.get_proof(&txn, vec![&key_3]).unwrap();
        assert_eq!(proof.nodes.len(), 3);
        assert!(proof.verify(&trie.root_hash_assert(&txn)));

        let proof = trie.get_proof(&txn, vec![&key_4, &key_2]).unwrap();
        assert_eq!(proof.nodes.len(), 4);
        assert!(proof.verify(&trie.root_hash_assert(&txn)));

        let proof = trie.get_proof(&txn, vec![&key_4]).unwrap();
        assert_eq!(proof.nodes.len(), 2);
        assert!(proof.verify(&trie.root_hash_assert(&txn)));
    }

    #[test]
    fn get_proof_values() {
        //          |
        //         cfb98
        //        /     |
        //       6    e0f6
        //      /  \
        //   ab9   f5a

        let key_1 = "cfb986f5a".parse().unwrap();
        let key_2 = "cfb98".parse().unwrap();
        let key_3 = "cfb98e0f6".parse().unwrap();
        let key_4 = "cfb98e0f7".parse().unwrap();
        let key_5 = "cfb98e".parse().unwrap();
        let key_6 = "ca".parse().unwrap();
        let key_7 = "b".parse().unwrap();

        let env = nimiq_database::volatile::VolatileDatabase::new(20).unwrap();
        let trie = MerkleRadixTrie::new(env.clone(), "database");
        let mut raw_txn = env.write_transaction();
        let mut txn: WriteTransactionProxy = (&mut raw_txn).into();

        trie.put(&mut txn, &key_1, 1u8).expect("complete trie");
        trie.put(&mut txn, &key_2, 2u8).expect("complete trie");
        trie.put(&mut txn, &key_3, 3u8).expect("complete trie");
        trie.put(&mut txn, &key_4, 4u8).expect("complete trie");
        trie.update_root(&mut txn).expect("complete trie");

        let proof_values = trie
            .get_proof(&txn, vec![&key_1, &key_2, &key_3])
            .unwrap()
            .verify_values(&trie.root_hash_assert(&txn), &[&key_1, &key_2, &key_3])
            .unwrap();
        assert_eq!(proof_values.len(), 3);
        assert_eq!(proof_values[&key_1], Some(vec![1]));
        assert_eq!(proof_values[&key_2], Some(vec![2]));
        assert_eq!(proof_values[&key_3], Some(vec![3]));

        let proof_values = trie
            .get_proof(&txn, vec![&key_1, &key_2])
            .unwrap()
            .verify_values(&trie.root_hash_assert(&txn), &[&key_1, &key_2])
            .unwrap();
        assert_eq!(proof_values.len(), 2);
        assert_eq!(proof_values[&key_1], Some(vec![1]));
        assert_eq!(proof_values[&key_2], Some(vec![2]));

        let proof_values = trie
            .get_proof(&txn, vec![&key_1, &key_3])
            .unwrap()
            .verify_values(&trie.root_hash_assert(&txn), &[&key_1, &key_3])
            .unwrap();
        assert_eq!(proof_values.len(), 2);
        assert_eq!(proof_values[&key_1], Some(vec![1]));
        assert_eq!(proof_values[&key_3], Some(vec![3]));

        // Empty nodes
        let proof_values = trie
            .get_proof(&txn, vec![&key_5])
            .unwrap()
            .verify_values(&trie.root_hash_assert(&txn), &[&key_5])
            .unwrap();
        assert_eq!(proof_values.len(), 1);
        assert_eq!(proof_values[&key_5], None);

        let proof_values = trie
            .get_proof(&txn, vec![&key_2, &key_5])
            .unwrap()
            .verify_values(&trie.root_hash_assert(&txn), &[&key_2, &key_5])
            .unwrap();
        assert_eq!(proof_values.len(), 2);
        assert_eq!(proof_values[&key_2], Some(vec![2]));
        assert_eq!(proof_values[&key_5], None);

        let proof_values = trie
            .get_proof(&txn, vec![&key_5, &key_6, &key_7])
            .unwrap()
            .verify_values(&trie.root_hash_assert(&txn), &[&key_5, &key_6, &key_7])
            .unwrap();
        assert_eq!(proof_values.len(), 3);
        assert_eq!(proof_values[&key_5], None);
        assert_eq!(proof_values[&key_6], None);
        assert_eq!(proof_values[&key_7], None);

        // Wrong values were proven.
        assert!(trie
            .get_proof(&txn, vec![&key_1, &key_2])
            .unwrap()
            .verify_values(&trie.root_hash_assert(&txn), &[&key_1, &key_3])
            .is_err());

        // Not all values were proven.
        assert!(trie
            .get_proof(&txn, vec![&key_1, &key_2])
            .unwrap()
            .verify_values(&trie.root_hash_assert(&txn), &[&key_1, &key_2, &key_3],)
            .is_err());

        // Not all empty values were proven.
        assert!(trie
            .get_proof(&txn, vec![&key_5])
            .unwrap()
            .verify_values(&trie.root_hash_assert(&txn), &[&key_5, &key_6],)
            .is_err());
        // Too many values were proven.
        assert!(trie
            .get_proof(&txn, vec![&key_1, &key_2, &key_3])
            .unwrap()
            .verify_values(&trie.root_hash_assert(&txn), &[&key_1, &key_2])
            .is_err());
    }

    #[test]
    fn hybrid_nodes_work() {
        let key_1 = "413f22".parse().unwrap();
        let key_2 = "413".parse().unwrap();
        let key_3 = "413f227fa".parse().unwrap();
        let key_4 = "413b391".parse().unwrap();
        let key_5 = "412324".parse().unwrap();

        let env = nimiq_database::volatile::VolatileDatabase::new(20).unwrap();
        let trie = MerkleRadixTrie::new(env.clone(), "database");
        let mut raw_txn = env.write_transaction();
        let mut txn: WriteTransactionProxy = (&mut raw_txn).into();

        let initial_hash = trie.root_hash_assert(&txn);
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

        assert_eq!(trie.root_hash_assert(&txn), initial_hash);
    }

    #[test]
    fn diffs_work() {
        use self::ValueChange::*;

        let key_1 = "413f22".parse().unwrap();
        let key_2 = "413".parse().unwrap();
        let key_3 = "413f227fa".parse().unwrap();
        let key_4 = "413b391".parse().unwrap();
        let key_5 = "412324".parse().unwrap();

        let env = nimiq_database::volatile::VolatileDatabase::new(20).unwrap();
        let trie = MerkleRadixTrie::new(env.clone(), "database");

        let mut raw_txn = env.write_transaction();
        let mut txn: WriteTransactionProxy = (&mut raw_txn).into();

        #[track_caller]
        fn assert_diff(got: TrieDiffBuilder, mut wanted: Vec<(KeyNibbles, ValueChange)>) {
            wanted.sort_by_key(|item| item.0.clone());
            let got: Vec<_> = got
                .changes
                .iter()
                .map(|(key, change)| (key.clone(), change.clone()))
                .collect();
            assert_eq!(got, wanted);
        }

        // Empty
        txn.start_recording();
        assert_diff(txn.stop_recording(), vec![]);

        // Puts
        txn.start_recording();
        trie.put::<u8>(&mut txn, &key_1, 1).expect("complete trie");
        trie.put::<u8>(&mut txn, &key_2, 2).expect("complete trie");
        trie.put::<u8>(&mut txn, &key_3, 3).expect("complete trie");
        trie.put::<u8>(&mut txn, &key_4, 4).expect("complete trie");
        assert_diff(
            txn.stop_recording(),
            vec![
                (key_1.clone(), Insert(vec![1])),
                (key_2.clone(), Insert(vec![2])),
                (key_3.clone(), Insert(vec![3])),
                (key_4.clone(), Insert(vec![4])),
            ],
        );

        // Non-existing delete
        txn.start_recording();
        trie.remove(&mut txn, &key_5);
        assert_diff(txn.stop_recording(), vec![]);

        // Delete
        txn.start_recording();
        trie.remove(&mut txn, &key_1);
        assert_diff(txn.stop_recording(), vec![(key_1.clone(), Delete(vec![1]))]);

        // Put followed by delete
        txn.start_recording();
        trie.put::<u8>(&mut txn, &key_1, 1).expect("complete trie");
        trie.remove(&mut txn, &key_1);
        assert_diff(txn.stop_recording(), vec![]);

        // Delete followed by put of the same value
        txn.start_recording();
        trie.remove(&mut txn, &key_2);
        trie.put::<u8>(&mut txn, &key_2, 2).expect("complete trie");
        assert_diff(txn.stop_recording(), vec![]);

        // Delete followed by put of a different value
        txn.start_recording();
        trie.remove(&mut txn, &key_3);
        trie.put::<u8>(&mut txn, &key_3, 33).expect("complete trie");
        assert_diff(
            txn.stop_recording(),
            vec![(key_3.clone(), Update(vec![3], vec![33]))],
        );

        // Two puts to the same existing key
        txn.start_recording();
        trie.put::<u8>(&mut txn, &key_2, 22).expect("complete trie");
        trie.put::<u8>(&mut txn, &key_2, 222)
            .expect("complete trie");
        assert_diff(
            txn.stop_recording(),
            vec![(key_2.clone(), Update(vec![2], vec![222]))],
        );

        // Two puts to the same non-existing key
        txn.start_recording();
        trie.put::<u8>(&mut txn, &key_1, 11).expect("complete trie");
        trie.put::<u8>(&mut txn, &key_1, 111)
            .expect("complete trie");
        assert_diff(
            txn.stop_recording(),
            vec![(key_1.clone(), Insert(vec![111]))],
        );

        // Puts to an existing key
        txn.start_recording();
        trie.put::<u8>(&mut txn, &key_4, 44).expect("complete trie");
        assert_diff(
            txn.stop_recording(),
            vec![(key_4.clone(), Update(vec![4], vec![44]))],
        );
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

        let env = nimiq_database::volatile::VolatileDatabase::new(20).unwrap();
        let trie = MerkleRadixTrie::new(env.clone(), "database");
        let mut raw_txn = env.write_transaction();
        let mut txn: WriteTransactionProxy = (&mut raw_txn).into();

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
    fn test_end_key_on_partial_tree() {
        let key_1: KeyNibbles = "5".parse().unwrap();
        let key_2: KeyNibbles = "413b39931".parse().unwrap();
        let key_3: KeyNibbles = "4".parse().unwrap();

        let proof_value_2 = TrieNode::new_leaf(key_2.clone(), vec![99]);
        let mut proof_root = TrieNode::new_root();
        proof_root
            .put_child(&proof_value_2.key, proof_value_2.hash_assert())
            .unwrap();

        let env = nimiq_database::volatile::VolatileDatabase::new(20).unwrap();
        let trie = MerkleRadixTrie::new_incomplete(env.clone(), "database");
        let mut raw_txn = env.write_transaction();
        let mut txn: WriteTransactionProxy = (&mut raw_txn).into();
        assert!(!trie.is_complete(&txn));

        trie.put_chunk(
            &mut txn,
            KeyNibbles::ROOT,
            TrieChunk::new(
                Some(key_3.clone()),
                Vec::new(),
                TrieProof::new(vec![TrieNode::new_root().into()], Default::default()),
            ),
            TrieNode::new_root().hash_assert(),
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
                TrieProof::new(vec![proof_root.clone().into()], Default::default()),
            ),
            proof_root.hash_assert(),
        )
        .unwrap();
        assert_eq!(trie.count_nodes(&txn), (0, 0, 0));
        assert!(!trie.is_complete(&txn));

        trie.put_chunk(
            &mut txn,
            key_3,
            TrieChunk::new(
                Some(key_1.clone()),
                vec![TrieItem::new(key_2, vec![99])],
                TrieProof::new(
                    vec![proof_value_2.clone().into(), proof_root.clone().into()],
                    Default::default(),
                ),
            ),
            proof_root.hash_assert(),
        )
        .unwrap();
        assert_eq!(trie.count_nodes(&txn), (0, 0, 1));
        assert!(!trie.is_complete(&txn));

        trie.put_chunk(
            &mut txn,
            key_1,
            TrieChunk::new(
                None,
                Vec::new(),
                TrieProof::new(
                    vec![proof_value_2.clone().into(), proof_root.clone().into()],
                    Default::default(),
                ),
            ),
            proof_root.hash_assert(),
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

        let env = nimiq_database::volatile::VolatileDatabase::new(20).unwrap();
        let original = MerkleRadixTrie::new(env.clone(), "original");
        let mut raw_txn = env.write_transaction();
        let mut txn: WriteTransactionProxy = (&mut raw_txn).into();

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
        let env = nimiq_database::volatile::VolatileDatabase::new(20).unwrap();
        let original = MerkleRadixTrie::new(env.clone(), "original");
        let trie = MerkleRadixTrie::new(env.clone(), "copy");
        let mut raw_txn = env.write_transaction();
        let mut txn: WriteTransactionProxy = (&mut raw_txn).into();

        let hash = original.root_hash_assert(&txn);

        let chunk = original.get_chunk_with_proof(&txn, KeyNibbles::ROOT.., 100);
        assert_eq!(
            trie.put_chunk(&mut txn, KeyNibbles::ROOT, chunk, hash),
            Err(MerkleRadixTrieError::TrieAlreadyComplete)
        );
    }

    #[test]
    fn partial_tree_put_chunks_manual() {
        let key_1 = "413f22".parse().unwrap();
        let key_2 = "413".parse().unwrap();
        let key_3 = "413f227fa".parse().unwrap();
        let key_4 = "413b391".parse().unwrap();
        let key_5 = "412324".parse().unwrap();

        let env = nimiq_database::volatile::VolatileDatabase::new(20).unwrap();
        let original = MerkleRadixTrie::new(env.clone(), "original");
        let trie = MerkleRadixTrie::new_incomplete(env.clone(), "copy");
        let mut raw_txn = env.write_transaction();
        let mut txn: WriteTransactionProxy = (&mut raw_txn).into();

        original
            .put(&mut txn, &key_1, 80085)
            .expect("complete trie");
        original.put(&mut txn, &key_2, 999).expect("complete trie");
        original.put(&mut txn, &key_3, 1337).expect("complete trie");
        original.put(&mut txn, &key_4, 6969).expect("complete trie");
        original.update_root(&mut txn).expect("complete trie");
        assert_eq!(original.count_nodes(&txn), (0, 2, 2));
        assert!(original.is_complete(&txn));

        let hash = original.root_hash_assert(&txn);
        let end = Some(KeyNibbles::ROOT);

        let start = end.unwrap();
        let chunk = original.get_chunk_with_proof(&txn, start.clone().., 0);
        trie.put_chunk(&mut txn, start, chunk.clone(), hash.clone())
            .unwrap();
        assert_eq!(trie.count_nodes(&txn), (0, 0, 0));
        assert!(!trie.is_complete(&txn));

        let start = chunk.end_key.unwrap();
        let chunk = original.get_chunk_with_proof(&txn, start.clone().., 1);
        trie.put_chunk(&mut txn, start, chunk.clone(), hash.clone())
            .unwrap();
        assert_eq!(trie.count_nodes(&txn), (0, 1, 0));
        assert!(!trie.is_complete(&txn));

        let start = chunk.end_key.unwrap();
        let chunk = original.get_chunk_with_proof(&txn, start.clone().., 0);
        trie.put_chunk(&mut txn, start, chunk.clone(), hash.clone())
            .unwrap();
        assert_eq!(trie.count_nodes(&txn), (0, 1, 0));
        assert!(!trie.is_complete(&txn));

        let start = chunk.end_key.unwrap();
        let chunk = original.get_chunk_with_proof(&txn, start.clone().., 2);
        trie.put_chunk(&mut txn, start, chunk.clone(), hash.clone())
            .unwrap();
        assert_eq!(trie.count_nodes(&txn), (0, 2, 1));
        assert!(!trie.is_complete(&txn));

        let start = chunk.end_key.unwrap();
        let chunk = original.get_chunk_with_proof(&txn, start.clone().., 2);
        trie.put_chunk(&mut txn, start, chunk, hash).unwrap();
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

        let env = nimiq_database::volatile::VolatileDatabase::new(20).unwrap();
        let original = MerkleRadixTrie::new(env.clone(), "original");
        let tries: Vec<_> = (1..5)
            .map(|i| {
                (
                    i,
                    MerkleRadixTrie::new_incomplete(env.clone(), &format!("copy{i}")),
                )
            })
            .collect();
        let mut raw_txn = env.write_transaction();
        let mut txn: WriteTransactionProxy = (&mut raw_txn).into();

        original
            .put(&mut txn, &key_1, 80085)
            .expect("complete trie");
        original.put(&mut txn, &key_2, 999).expect("complete trie");
        original.put(&mut txn, &key_3, 1337).expect("complete trie");
        original.put(&mut txn, &key_4, 6969).expect("complete trie");
        original.update_root(&mut txn).expect("complete trie");
        assert_eq!(original.count_nodes(&txn), (0, 2, 2));
        assert!(original.is_complete(&txn));

        let hash = original.root_hash_assert(&txn);
        for (chunk_size, trie) in tries {
            let mut next_start = KeyNibbles::ROOT;
            for _ in 0..10 {
                let start = next_start;
                let chunk = original.get_chunk_with_proof(&txn, start.clone().., chunk_size);

                trie.put_chunk(&mut txn, start, chunk.clone(), hash.clone())
                    .unwrap();

                if trie.is_complete(&txn) {
                    break;
                }
                next_start = chunk.end_key.unwrap();
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
    fn partial_tree_put_chunks_of_different_sizes_2() {
        let key_1 = "1a".parse().unwrap();
        let key_2 = "1b1a".parse().unwrap();
        let key_3 = "1b1b".parse().unwrap();
        let key_4 = "1c".parse().unwrap();
        let key_5 = "81".parse().unwrap();

        let env = nimiq_database::volatile::VolatileDatabase::new(20).unwrap();
        let original = MerkleRadixTrie::new(env.clone(), "original");
        let tries: Vec<_> = (1..5)
            .map(|i| {
                (
                    i,
                    MerkleRadixTrie::new_incomplete(env.clone(), &format!("copy{i}")),
                )
            })
            .collect();
        let mut raw_txn = env.write_transaction();
        let mut txn: WriteTransactionProxy = (&mut raw_txn).into();

        original
            .put(&mut txn, &key_1, 80085)
            .expect("complete trie");
        original.put(&mut txn, &key_2, 999).expect("complete trie");
        original.put(&mut txn, &key_3, 1337).expect("complete trie");
        original.put(&mut txn, &key_4, 6969).expect("complete trie");
        original.put(&mut txn, &key_5, 2969).expect("complete trie");
        original.update_root(&mut txn).expect("complete trie");
        assert_eq!(original.count_nodes(&txn), (2, 0, 5));
        assert!(original.is_complete(&txn));

        let hash = original.root_hash_assert(&txn);
        for (chunk_size, trie) in tries {
            let mut next_start = KeyNibbles::ROOT;
            for _ in 0..10 {
                let start = next_start;
                let chunk = original.get_chunk_with_proof(&txn, start.clone().., chunk_size);

                trie.put_chunk(&mut txn, start, chunk.clone(), hash.clone())
                    .unwrap();

                if trie.is_complete(&txn) {
                    break;
                }
                next_start = chunk.end_key.unwrap();
            }
            assert!(trie.is_complete(&txn));
            assert_eq!(trie.count_nodes(&txn), (2, 0, 5));

            assert_eq!(trie.get(&txn, &key_1).expect("complete trie"), Some(80085));
            assert_eq!(trie.get(&txn, &key_2).expect("complete trie"), Some(999));
            assert_eq!(trie.get(&txn, &key_3).expect("complete trie"), Some(1337));
            assert_eq!(trie.get(&txn, &key_4).expect("complete trie"), Some(6969));
            assert_eq!(trie.get(&txn, &key_5).expect("complete trie"), Some(2969));
        }
    }

    #[test]
    fn partial_tree_remove_chunks_of_different_sizes() {
        let key_1 = "413f22".parse().unwrap();
        let key_2 = "413".parse().unwrap();
        let key_3 = "413f227fa".parse().unwrap();
        let key_4 = "413b391".parse().unwrap();
        let key_5 = "412324".parse().unwrap();

        let env = nimiq_database::volatile::VolatileDatabase::new(20).unwrap();
        let original = MerkleRadixTrie::new(env.clone(), "original");
        let tries: Vec<_> = (1..5)
            .map(|i| {
                let trie = MerkleRadixTrie::new(env.clone(), &format!("copy{i}"));
                let mut raw_txn = env.write_transaction();
                let mut txn: WriteTransactionProxy = (&mut raw_txn).into();
                trie.put(&mut txn, &key_1, 80085).expect("complete trie");
                trie.put(&mut txn, &key_2, 999).expect("complete trie");
                trie.put(&mut txn, &key_3, 1337).expect("complete trie");
                trie.put(&mut txn, &key_4, 6969).expect("complete trie");
                trie.update_root(&mut txn).expect("complete trie");
                assert_eq!(trie.count_nodes(&txn), (0, 2, 2));
                assert!(trie.is_complete(&txn));
                raw_txn.commit();

                (i, trie)
            })
            .collect();
        let mut raw_txn = env.write_transaction();
        let mut txn: WriteTransactionProxy = (&mut raw_txn).into();

        original
            .put(&mut txn, &key_1, 80085)
            .expect("complete trie");
        original.put(&mut txn, &key_2, 999).expect("complete trie");
        original.put(&mut txn, &key_3, 1337).expect("complete trie");
        original.put(&mut txn, &key_4, 6969).expect("complete trie");
        original.update_root(&mut txn).expect("complete trie");
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
    fn trie_cache_consistency() {
        let key_1 = "413f22".parse().unwrap();
        let key_2 = "413".parse().unwrap();
        let key_3 = "413f227fa".parse().unwrap();
        let key_4 = "413b391".parse().unwrap();

        let env = nimiq_database::volatile::VolatileDatabase::new(20).unwrap();
        let original = MerkleRadixTrie::new(env.clone(), "original");

        let mut raw_txn = env.write_transaction();
        let mut txn: WriteTransactionProxy = (&mut raw_txn).into();

        original
            .put(&mut txn, &key_1, 80085)
            .expect("complete trie");
        original.put(&mut txn, &key_2, 999).expect("complete trie");
        original.put(&mut txn, &key_3, 1337).expect("complete trie");
        original.put(&mut txn, &key_4, 6969).expect("complete trie");
        original.update_root(&mut txn).expect("complete trie");
        assert_eq!(original.count_nodes(&txn), (0, 2, 2));
        assert!(original.is_complete(&txn));

        raw_txn.abort();

        let mut raw_txn = env.write_transaction();
        let txn: WriteTransactionProxy = (&mut raw_txn).into();
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

        let env = nimiq_database::volatile::VolatileDatabase::new(20).unwrap();
        let original = MerkleRadixTrie::new(env.clone(), "original");

        let mut raw_txn = env.write_transaction();
        let mut txn: WriteTransactionProxy = (&mut raw_txn).into();

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
        original.update_root(&mut txn).expect("complete trie");
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

        let env = nimiq_database::volatile::VolatileDatabase::new(20).unwrap();
        let original = MerkleRadixTrie::new(env.clone(), "original");

        let mut raw_txn = env.write_transaction();
        let mut txn: WriteTransactionProxy = (&mut raw_txn).into();

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

        let env = nimiq_database::volatile::VolatileDatabase::new(20).unwrap();
        let original = MerkleRadixTrie::new(env.clone(), "original");

        let mut raw_txn = env.write_transaction();
        let mut txn: WriteTransactionProxy = (&mut raw_txn).into();

        original
            .put(&mut txn, &key_1, 80085)
            .expect("complete trie");
        original.put(&mut txn, &key_2, 999).expect("complete trie");
        original.put(&mut txn, &key_3, 1337).expect("complete trie");
        original.put(&mut txn, &key_4, 6969).expect("complete trie");
        original.update_root(&mut txn).expect("complete trie");
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

        let env = nimiq_database::volatile::VolatileDatabase::new(20).unwrap();
        let original = MerkleRadixTrie::new(env.clone(), "original");

        let mut raw_txn = env.write_transaction();
        let mut txn: WriteTransactionProxy = (&mut raw_txn).into();
        assert!(original.is_complete(&txn));

        original
            .put(&mut txn, &key_1, 80085)
            .expect("complete trie");
        original.put(&mut txn, &key_2, 999).expect("complete trie");
        original.put(&mut txn, &key_3, 1337).expect("complete trie");
        original.put(&mut txn, &key_4, 6969).expect("complete trie");
        original.update_root(&mut txn).expect("complete trie");
        assert_eq!(original.count_nodes(&txn), (0, 2, 2));
        assert!(original.is_complete(&txn));

        let hash = original.root_hash_assert(&txn);
        let chunk = original.get_chunk_with_proof(&txn, key_1.clone().., 2);

        original.remove_chunk(&mut txn, key_1.clone()).unwrap();
        assert_eq!(original.count_nodes(&txn), (0, 1, 1));
        assert!(!original.is_complete(&txn));

        original.put_chunk(&mut txn, key_1, chunk, hash).unwrap();
        assert_eq!(original.count_nodes(&txn), (0, 2, 2));
        assert!(original.is_complete(&txn));
    }
}
