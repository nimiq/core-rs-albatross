use std::{
    cmp,
    collections::{BTreeMap, BTreeSet},
    marker::PhantomData,
    mem, ops,
};

use log::error;
use nimiq_database::{
    mdbx::{IntoIterProxy, MdbxDatabase, MdbxReadTransaction},
    traits::{Database, ReadCursor, ReadTransaction, WriteTransaction},
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
    transaction::{OldValue, TransactionExt as _, TrieTable},
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
pub struct MerkleRadixTrie<T: TrieTable> {
    table: T,
}

/// Counts the number of updates performed.
#[derive(Default)]
struct CountUpdates {
    branches: i64,
    hybrids: i64,
    leaves: i64,
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
    fn counter_mut(&mut self, kind: Option<TrieNodeKind>) -> Option<&mut i64> {
        match kind {
            None => None,
            Some(TrieNodeKind::Root) => None,
            Some(TrieNodeKind::Branch) => Some(&mut self.branches),
            Some(TrieNodeKind::Hybrid) => Some(&mut self.hybrids),
            Some(TrieNodeKind::Leaf) => Some(&mut self.leaves),
        }
    }
    fn update_root_data(self, root_data: &mut RootData) {
        let apply_difference = |data: &mut u64, diff: i64| {
            if diff >= 0 {
                *data += diff as u64;
            } else {
                *data -= (-diff) as u64;
            }
        };
        apply_difference(&mut root_data.num_branches, self.branches);
        apply_difference(&mut root_data.num_hybrids, self.hybrids);
        apply_difference(&mut root_data.num_leaves, self.leaves);
    }
}

impl<T: TrieTable> MerkleRadixTrie<T> {
    /// Start a new Merkle Radix Trie with the given Environment and the given name.
    pub fn new(db: &MdbxDatabase, table: T) -> Self {
        Self::new_impl(db, table, false)
    }

    pub fn new_incomplete(db: &MdbxDatabase, table: T) -> Self {
        Self::new_impl(db, table, true)
    }

    fn new_impl(db: &MdbxDatabase, table: T, incomplete: bool) -> Self {
        let tree = MerkleRadixTrie { table };

        db.create_regular_table(&tree.table);

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
        txn.clear_table(&self.table);
        self.init_root(txn, true);
    }

    /// Prints a human friendly version of the subtrie for debugging.
    /// Not to be used on large trees!
    pub fn debug_print(&self, txn: &MdbxReadTransaction) {
        let missing_range = self.get_missing_range(txn);
        println!("-> ROOT");
        self.debug_print_subtrie(txn, &KeyNibbles::ROOT, &missing_range, 2)
    }

    /// Prints a human friendly version of the subtrie for debugging.
    /// Not to be used on large trees!
    pub fn debug_print_subtrie(
        &self,
        txn: &MdbxReadTransaction,
        key: &KeyNibbles,
        missing_range: &Option<ops::RangeFrom<KeyNibbles>>,
        depth: usize,
    ) {
        let prefix = "  ".repeat(depth);
        let node = if let Some(node) = self.get_node(txn, key) {
            node
        } else {
            println!("{prefix}-> MISSING");
            return;
        };

        for child in node.iter_children() {
            println!("{}-> {} ({})", prefix, child.suffix, child.has_hash());
            if let Ok(subkey) = child.key(key, missing_range) {
                self.debug_print_subtrie(txn, &subkey, missing_range, depth + 2);
            } else {
                println!("{prefix}  -> MISSING");
            }
        }
    }

    /// Returns the root hash of the Merkle Radix Trie.
    pub fn root_hash_assert(&self, txn: &MdbxReadTransaction) -> Blake2bHash {
        let root = self.get_root(txn).unwrap();

        // We should refuse to return a hash for a completely empty but incomplete trie.
        assert!(
            self.is_complete(txn) || root.has_children(),
            "Cannot compute hash on an empty, incomplete trie"
        );

        root.hash_assert()
    }

    /// Returns the root hash of the Merkle Radix Trie if the trie is complete.
    pub fn root_hash(&self, txn: &MdbxReadTransaction) -> Option<Blake2bHash> {
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

    pub fn is_complete(&self, txn: &MdbxReadTransaction) -> bool {
        self.get_root(txn)
            .unwrap()
            .root_data
            .unwrap()
            .incomplete
            .is_none()
    }

    /// Returns the number of branch nodes in the Merkle Radix Trie.
    pub fn num_branches(&self, txn: &MdbxReadTransaction) -> u64 {
        self.get_root(txn).unwrap().root_data.unwrap().num_branches
    }

    /// Returns the number of hybrid nodes in the Merkle Radix Trie.
    pub fn num_hybrids(&self, txn: &MdbxReadTransaction) -> u64 {
        self.get_root(txn).unwrap().root_data.unwrap().num_hybrids
    }

    /// Returns the number of leaf nodes in the Merkle Radix Trie.
    pub fn num_leaves(&self, txn: &MdbxReadTransaction) -> u64 {
        self.get_root(txn).unwrap().root_data.unwrap().num_leaves
    }

    #[cfg(test)]
    fn count_nodes(&self, txn: &MdbxReadTransaction) -> (u64, u64, u64) {
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

    fn get_node(&self, txn: &MdbxReadTransaction, key: &KeyNibbles) -> Option<TrieNode> {
        txn.get_node(&self.table, key)
    }

    fn put_node(&self, txn: &mut WriteTransactionProxy, node: &TrieNode, old_value: OldValue) {
        txn.put_node(&self.table, node, old_value)
    }

    fn remove_node(&self, txn: &mut WriteTransactionProxy, key: &KeyNibbles, old_value: OldValue) {
        txn.remove_node(&self.table, key, old_value)
    }

    /// Get the value at the given key. If there's no leaf or hybrid node at the given key then it
    /// returns None.
    pub fn get<D: Deserialize>(
        &self,
        txn: &MdbxReadTransaction,
        key: &KeyNibbles,
    ) -> Result<Option<D>, IncompleteTrie> {
        let missing_range = self.get_missing_range(txn);
        if !self.is_within_complete_part(key, &missing_range) {
            return Err(IncompleteTrie);
        }
        Ok(self
            .get_raw(txn, key)
            .map(|v| D::deserialize_from_vec(&v).unwrap()))
    }

    fn get_raw(&self, txn: &MdbxReadTransaction, key: &KeyNibbles) -> Option<Vec<u8>> {
        self.get_node(txn, key)?.value
    }

    /// Insert a value into the Merkle Radix Trie at the given key. If the key already exists then
    /// it will overwrite it. You can't use this function to check the existence of a given key.
    pub fn put<S: Serialize>(
        &self,
        txn: &mut WriteTransactionProxy,
        key: &KeyNibbles,
        value: S,
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
        debug_assert!(
            !key.is_empty(),
            "empty (ROOT) key must be rejected at the put_chunk boundary",
        );

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
                // `put_raw` rejects empty keys at its entry, so `cur_node` here never has
                // `root_data.is_some()`; `put_value` cannot return `RootCantHaveValue`.
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
                // The matching child is a stump. The key is in the complete part, so it can't be
                // below the stump and they need a common parent instead
                Err(MerkleRadixTrieError::ChildIsStump) => {
                    let (stump_key, stump_hash) = Self::child_entry(&cur_node, key);
                    let common_prefix = stump_key.common_prefix(key);

                    let mut new_node = TrieNode::new_leaf(key.clone(), value);
                    if common_prefix == *key {
                        // The new node is the parent of the stump. Thus it needs to be a hybrid node
                        new_node.put_child(&stump_key, stump_hash).unwrap();
                        self.put_node(txn, &new_node, OldValue::None);

                        count_updates = CountUpdates {
                            hybrids: 1,
                            ..Default::default()
                        };
                        root_path.push(cur_node);
                        root_path.push(new_node);
                    } else {
                        // The new node is a sibling of the stump. We insert a new branch node as
                        // the parent of both
                        self.put_node(txn, &new_node, OldValue::None);

                        let mut new_parent = TrieNode::new_empty(common_prefix);
                        new_parent.put_child(&stump_key, stump_hash).unwrap();
                        new_parent
                            .put_child(&new_node.key, new_node.hash_assert())
                            .unwrap();
                        self.put_node(txn, &new_parent, OldValue::None);

                        count_updates = CountUpdates {
                            branches: 1,
                            leaves: 1,
                            ..Default::default()
                        };
                        root_path.push(cur_node);
                        root_path.push(new_parent);
                    }
                    break;
                }
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

                    cur_node = txn.get_node(&self.table, &child_key).unwrap();
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
        // The root node is loaded below and the descent loop's invariant is
        // `node.key != key`; the ROOT key would violate that on the first
        // iteration. Reject it cleanly so `apply_diff` returns an error
        // instead of panicking on an attacker-supplied diff entry.
        if key.is_empty() {
            return Err(MerkleRadixTrieError::InvalidChunk(
                "diff key must not be the root key",
            ));
        }

        let mut node = self
            .get_root(txn)
            .expect("The Merkle Radix Trie didn't have a root node!");

        let mut root_path = Vec::new();
        let mut count_updates = CountUpdates::default();
        let new_stump;
        // Descend down the tree and collect nodes to be updated.
        loop {
            // Check that the key we are trying to update can exist in this part of the tree.
            if !node.key.is_prefix_of(key) {
                return Err(MerkleRadixTrieError::ChildDoesNotExist);
            }

            let idx = node.child_index(key)?;
            let Some(child) = &node.children[idx] else {
                // There is no child for the key yet, so we add a new stump for it.
                // Update this node prior to `update_keys` (which only updates the other nodes in the root path)
                let prev_kind = node.kind();
                node.put_child_no_hash(key)?;
                count_updates.apply_update(prev_kind, node.kind());
                self.put_node(txn, &node, OldValue::Unchanged);

                root_path.push(node);
                new_stump = true;
                break;
            };

            let child_key = node
                .key
                .checked_add(&child.suffix)
                .ok_or(MerkleRadixTrieError::WrongPrefix)?;
            let common_prefix = child_key.common_prefix(key);

            if !self.is_within_complete_part(&common_prefix, missing_range) {
                // The key is in the subtree of a stump. We cannot continue further down as the
                // next node is outside our trie, so we only mark the stump as changed. Its suffix
                // stays unchanged, since it still points to the missing part of the trie
                node.put_child(&child_key, Blake2bHash::default())?;
                self.put_node(txn, &node, OldValue::Unchanged);

                root_path.push(node);
                new_stump = false;
                break;
            }

            if common_prefix == child_key {
                // The child is in the complete part and a prefix of the key, continue there
                let child = self
                    .get_node(txn, &child_key)
                    .expect("Child should be present");
                let parent = mem::replace(&mut node, child);
                root_path.push(parent);
                continue;
            }

            // The key and the child diverge in the complete part of the trie. We insert a new
            // branch node as the parent of both, with a new stump for the key
            let mut new_parent = TrieNode::new_empty(common_prefix);
            new_parent.put_child(&child_key, child.hash.clone())?;
            new_parent.put_child_no_hash(key)?;
            self.put_node(txn, &new_parent, OldValue::None);
            count_updates.branches += 1;

            root_path.push(node);
            root_path.push(new_parent);
            new_stump = true;
            break;
        }

        self.update_keys(txn, root_path, count_updates);

        Ok(new_stump)
    }

    pub fn get_chunk_with_proof(
        &self,
        txn: &MdbxReadTransaction,
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

        // First, remove the stumps from the nodes on the path to the start key
        let mut path: Vec<TrieNode> = vec![];
        let mut count_updates = CountUpdates::default();

        loop {
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
            // The child on the path of the key can be a stump too, e.g. after its parent
            // branch was merged into it
            if let Some(digit) = cur_digit {
                let child = &mut cur_node.children[digit];
                if child
                    .as_ref()
                    .is_some_and(|c| c.is_stump(&cur_node.key, &missing_range))
                {
                    *child = None;
                }
            }
            count_updates.apply_update(prev_kind, cur_node.kind());

            // Continue with the child on the path to the start key, if there is one
            let next = if cur_node.key.len() == keys.start.len() {
                None
            } else {
                cur_node
                    .child_key(&keys.start, &missing_range)
                    .ok()
                    .filter(|child_key| child_key.is_prefix_of(&keys.start))
            };
            path.push(cur_node);
            match next {
                Some(child_key) => cur_node = self.get_node(txn, &child_key).unwrap(),
                None => break,
            }
        }

        // Then, going back up, remove the nodes that lost all their children and merge the ones
        // that are left with a single child into it
        let mut changed_child = None;
        while let Some(mut node) = path.pop() {
            if let Some(child_key) = changed_child.take() {
                // Mark the changed child as dirty
                node.put_child(&child_key, Blake2bHash::default()).unwrap();
            }
            match path.last_mut() {
                Some(parent) if node.value.is_none() && node.iter_children().count() <= 1 => {
                    self.remove_node(txn, &node.key, OldValue::Unchanged);
                    count_updates.apply_update(node.kind(), None);

                    let prev_parent_kind = parent.kind();
                    if node.has_children() {
                        Self::replace_with_only_child(parent, &node);
                    } else {
                        parent.remove_child(&node.key).unwrap();
                    }
                    count_updates.apply_update(prev_parent_kind, parent.kind());
                }
                Some(_) => {
                    self.put_node(txn, &node, OldValue::Unchanged);
                    changed_child = Some(node.key);
                }
                None => {
                    mem::take(&mut count_updates).update_root_data(
                        node.root_data.as_mut().expect("root node needs root data"),
                    );
                    self.put_node(txn, &node, OldValue::Unchanged);
                }
            }
        }
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

        if let Some(end_key) = &chunk.end_key
            && start_key > *end_key
        {
            return Err(MerkleRadixTrieError::InvalidChunk("invalid keys"));
        }
        if !chunk.items.is_empty() {
            if start_key > chunk.items[0].key {
                return Err(MerkleRadixTrieError::InvalidChunk(
                    "first key is inconsistent with key range",
                ));
            }

            // The root node holds no value, so a ROOT-keyed item would panic in `put_raw`.
            // ROOT is the minimum key, so checking `items[0]` suffices given the sort check below.
            if chunk.items[0].key.is_empty() {
                return Err(MerkleRadixTrieError::InvalidChunk(
                    "item key must not be the root key",
                ));
            }

            let last_item_key = chunk.items.last().unwrap().key.clone();
            if let Some(end_key) = &chunk.end_key
                && last_item_key >= *end_key
            {
                return Err(MerkleRadixTrieError::InvalidChunk(
                    "last key is inconsistent with key range",
                ));
            }
            let end_key_range = chunk.end_key.clone().map(|end_key| end_key..);
            for proof_node in chunk.proof.nodes.iter() {
                for child in proof_node.iter_children() {
                    if let Ok(key) = child.key(&proof_node.key, &end_key_range)
                        && key > last_item_key
                    {
                        return Err(MerkleRadixTrieError::InvalidChunk(
                            "invalid keys end, found stumps",
                        ));
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
            if let Some(last_proof_node) = chunk.proof.nodes.first()
                && last_proof_node.key == *end_key
            {
                is_valid_end = true;
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
            if let Some(last_proof_node) = chunk.proof.nodes.first()
                && last_proof_node.key >= *end_key
            {
                return Err(MerkleRadixTrieError::InvalidChunk(
                    "end key inconsistent with last proof node",
                ));
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
    ///
    /// Everything from `start_key` on becomes part of the missing range again. The removed
    /// subtrees are replaced by stumps, which keep the hashes of the subtrees where they're known.
    pub fn remove_chunk(
        &self,
        txn: &mut WriteTransactionProxy,
        start_key: KeyNibbles,
    ) -> Result<(), MerkleRadixTrieError> {
        let missing_range = self.get_missing_range(txn);

        // Nothing to do if the trie is already missing everything from `start_key` on
        if missing_range
            .as_ref()
            .is_some_and(|range| range.start <= start_key)
        {
            return Ok(());
        }

        // Update the hashes first, so that the stumps keep the hashes of their subtrees
        let _ = self.update_hashes(txn, &KeyNibbles::ROOT, &missing_range);

        // Walk down the path to the start key and remove every subtree from the start key on,
        // keeping only its entry in the parent as a stump
        let mut count_updates = CountUpdates::default();
        let mut cur_node = self
            .get_root(txn)
            .expect("Merkle Radix Trie must have a root node!");
        loop {
            let digit = start_key.get(cur_node.key.len()).unwrap_or(0);
            let mut next = None;
            for child in cur_node.children[digit..].iter().flatten() {
                let child_key = cur_node
                    .key
                    .checked_add(&child.suffix)
                    .ok_or(MerkleRadixTrieError::WrongPrefix)?;
                if child_key < start_key {
                    // Only a child on the path to the start key can have smaller keys below it
                    if child_key.is_prefix_of(&start_key) {
                        next = Some(child_key);
                    }
                } else if !child.is_stump(&cur_node.key, &missing_range) {
                    self.remove_subtree(txn, &child_key, &missing_range, &mut count_updates);
                }
            }
            match next {
                Some(child_key) => {
                    cur_node = self
                        .get_node(txn, &child_key)
                        .expect("Child should be present")
                }
                None => break,
            }
        }

        let mut root_node = self.get_root(txn).unwrap();
        let root_data = root_node.root_data.as_mut().unwrap();
        count_updates.update_root_data(root_data);
        root_data.incomplete = Some(start_key..);
        self.put_node(txn, &root_node, OldValue::Unchanged);

        Ok(())
    }

    /// Removes the node with the given key and all nodes below it from the database.
    fn remove_subtree(
        &self,
        txn: &mut WriteTransactionProxy,
        key: &KeyNibbles,
        missing_range: &Option<ops::RangeFrom<KeyNibbles>>,
        count_updates: &mut CountUpdates,
    ) {
        let mut stack = vec![key.clone()];
        while let Some(key) = stack.pop() {
            let node = self.get_node(txn, &key).expect("Child should be present");
            // Stumps aren't stored, so there's nothing to remove for them
            for child in node.iter_children() {
                if let Ok(child_key) = child.key(&node.key, missing_range) {
                    stack.push(child_key);
                }
            }
            count_updates.apply_update(node.kind(), None);
            self.remove_node(txn, &node.key, node.value.into());
        }
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

                        // Let the parent point to the node's only child instead
                        let mut parent = root_path.pop().expect("Non-root node must have a parent");
                        Self::replace_with_only_child(&mut parent, &cur_node);
                        self.put_node(txn, &parent, OldValue::Unchanged);

                        root_path.push(parent);

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

                // Let the node's parent point to its only child instead
                let mut grandparent = root_path.pop().expect("Non-root node must have a parent");
                Self::replace_with_only_child(&mut grandparent, &parent_node);
                self.put_node(txn, &grandparent, OldValue::Unchanged);

                root_path.push(grandparent);

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
        txn: &MdbxReadTransaction,
        mut keys: Vec<&KeyNibbles>,
    ) -> Result<TrieProof, IncompleteTrie> {
        if keys.is_empty() {
            return Ok(TrieProof::new(
                vec![self.get_root(txn).unwrap().into()],
                Default::default(),
            ));
        }

        // We sort the keys in post-order.
        keys.sort_by(|&k1, &k2| k1.post_order_cmp(k2));

        let missing_range = self.get_missing_range(txn);
        if let Some(missing) = &missing_range
            && keys.iter().any(|&key| missing.contains(key))
        {
            return Err(IncompleteTrie);
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
            // The ROOT key can never hold a value; routing it into the complete
            // branch (`put_raw`) panics on `put_value().unwrap()` (RootCantHaveValue),
            // and the only guard there is a release-stripped `debug_assert!`. The
            // missing branch is already guarded; guard the complete branch too so an
            // attacker-supplied diff entry cannot crash the node.
            if key.is_empty() {
                return Err(MerkleRadixTrieError::InvalidChunk(
                    "diff key must not be the root key",
                ));
            }
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
                    // A reverted chunk may have replaced the stump that was added for this key
                    // with one for a larger part of the trie, so we never remove it. We only mark
                    // it as changed; the next chunk replaces it anyway
                    RevertDiffValue::Remove | RevertDiffValue::UpdateStump => {
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
    pub fn get_missing_range(
        &self,
        txn: &MdbxReadTransaction,
    ) -> Option<ops::RangeFrom<KeyNibbles>> {
        self.get_root(txn)
            .expect("trie needs root node")
            .root_data
            .expect("root node needs root data")
            .incomplete
    }

    /// Returns the root node, if there is one.
    fn get_root(&self, txn: &MdbxReadTransaction) -> Option<TrieNode> {
        self.get_node(txn, &KeyNibbles::ROOT)
    }

    /// Returns the key and hash of the child of `node` at the position of `key`. The child may be
    /// a stump.
    fn child_entry(node: &TrieNode, key: &KeyNibbles) -> (KeyNibbles, Blake2bHash) {
        let child = node.children[node.child_index(key).expect("Key must be below the node")]
            .as_ref()
            .expect("Child must exist");
        let child_key = node
            .key
            .checked_add(&child.suffix)
            .expect("Child key must fit");
        (child_key, child.hash.clone())
    }

    /// Replaces `node` in its parent by the node's only child. Only the child's entry is moved,
    /// so the child may be a stump.
    fn replace_with_only_child(parent: &mut TrieNode, node: &TrieNode) {
        let child = node.iter_children().next().expect("Node must have a child");
        let child_key = node
            .key
            .checked_add(&child.suffix)
            .expect("Child key must fit");
        parent
            .put_child(&child_key, child.hash.clone())
            .expect("Child must be below the parent");
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
    fn get_predecessor(&self, txn: &MdbxReadTransaction, key: &KeyNibbles) -> Option<KeyNibbles> {
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
                Some(child) => match child.key(&cur_node.key, &missing_range) {
                    Ok(child_key) => cur_node = self.get_node(txn, &child_key).unwrap(),
                    // Everything below a stump comes after the given key, we're done
                    Err(_) => break,
                },
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
        txn: &MdbxReadTransaction,
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

    pub fn iter_nodes<'txn, D: Deserialize>(
        &self,
        txn: &'txn MdbxReadTransaction,
        start_key: &KeyNibbles,
        end_key: &KeyNibbles,
    ) -> TrieNodeIter<'txn, T, D> {
        assert_eq!(
            start_key.len(),
            end_key.len(),
            "Start and end keys should have the same length"
        );
        TrieNodeIter::new(&self.table, txn, start_key, end_key.clone())
    }
}

/// This iterator is meant to start at `start_key` and finish at `end_key`, both of these are inclusive.
pub struct TrieNodeIter<'txn, T: TrieTable, D> {
    iter: IntoIterProxy<'txn, T>,
    end_key: KeyNibbles,
    _type: PhantomData<D>,
}

impl<T: TrieTable, D> TrieNodeIter<'_, T, D> {
    /// This iterator is meant to start at `start_key` and finish at `end_key`, both of these are inclusive.
    fn new(
        table: &T,
        txn: &MdbxReadTransaction,
        start_key: &KeyNibbles,
        end_key: KeyNibbles,
    ) -> Self {
        let cursor = txn.cursor(table);

        Self {
            iter: cursor.into_iter_from(start_key),
            end_key,
            _type: PhantomData,
        }
    }
}

impl<T: TrieTable, D: Deserialize> Iterator for TrieNodeIter<'_, T, D> {
    type Item = D;

    fn next(&mut self) -> Option<Self::Item> {
        let (k, v) = self.iter.next()?;

        if k <= self.end_key {
            return D::deserialize_from_vec(&v.value?).ok();
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use nimiq_database::declare_table;
    use nimiq_primitives::trie::trie_diff::{TrieDiffBuilder, ValueChange};
    use nimiq_test_log::test;

    use super::*;

    declare_table!(TestTrie, "database", KeyNibbles => TrieNode);
    declare_table!(TestTrieCopy, "copy", KeyNibbles => TrieNode);

    #[test]
    fn get_put_remove_works() {
        let key_1 = "413f22b3e".parse().unwrap();
        let key_2 = "413b39931".parse().unwrap();
        let key_3 = "413b397fa".parse().unwrap();
        let key_4 = "cfb986f5a".parse().unwrap();

        let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
        let trie = MerkleRadixTrie::new(&env, TestTrie);
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

        let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
        let trie = MerkleRadixTrie::new(&env, TestTrie);
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
    fn get_proof_accepts_empty_key_sets() {
        let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
        let trie = MerkleRadixTrie::new(&env, TestTrie);
        let mut raw_txn = env.write_transaction();
        let mut txn: WriteTransactionProxy = (&mut raw_txn).into();

        let key: KeyNibbles = "cfb986f5a".parse().unwrap();
        trie.put(&mut txn, &key, 9u8).expect("complete trie");
        trie.update_root(&mut txn).expect("complete trie");

        let proof = trie.get_proof(&txn, Vec::new()).unwrap();
        assert_eq!(proof.nodes.len(), 1);
        assert!(proof.verify(&trie.root_hash_assert(&txn)));

        let proof_values = proof
            .verify_values(&trie.root_hash_assert(&txn), &[])
            .unwrap();
        assert!(proof_values.is_empty());
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

        let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
        let trie = MerkleRadixTrie::new(&env, TestTrie);
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

        let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
        let trie = MerkleRadixTrie::new(&env, TestTrie);
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

        let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
        let trie = MerkleRadixTrie::new(&env, TestTrie);

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

        let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
        let trie = MerkleRadixTrie::new(&env, TestTrie);
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

        let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
        let trie = MerkleRadixTrie::new_incomplete(&env, TestTrie);
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

        let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
        let original = MerkleRadixTrie::new(&env, TestTrie);
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
        let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
        let original = MerkleRadixTrie::new(&env, TestTrie);
        let trie = MerkleRadixTrie::new(&env, TestTrieCopy);
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
    fn put_chunk_rejects_root_keyed_item() {
        // A ROOT-keyed item used to reach `put_raw` and panic on
        // `put_value(...).unwrap()`; `put_chunk` must reject it as `InvalidChunk`.
        let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
        let trie = MerkleRadixTrie::new_incomplete(&env, TestTrie);
        let mut raw_txn = env.write_transaction();
        let mut txn: WriteTransactionProxy = (&mut raw_txn).into();
        assert!(!trie.is_complete(&txn));

        let proof_root = TrieNode::new_root();
        let expected_hash = proof_root.hash_assert();
        let proof = TrieProof::new(vec![proof_root.into()], Default::default());

        let malicious_chunk = TrieChunk::new(
            None,
            vec![TrieItem::new(
                KeyNibbles::ROOT,
                vec![0xab, 0xbc, 0xcd, 0xde],
            )],
            proof,
        );

        match trie.put_chunk(&mut txn, KeyNibbles::ROOT, malicious_chunk, expected_hash) {
            Err(MerkleRadixTrieError::InvalidChunk(_)) => {}
            other => panic!("expected InvalidChunk(_), got {:?}", other),
        }
    }

    #[test]
    fn put_chunk_rejects_oversized_child_suffix() {
        // A `TrieNodeChild.suffix` deserialized from the network used to cause an
        // out-of-bounds slice panic inside `KeyNibbles::Add` when concatenated with the
        // proof node's parent key. Both operands pass the `length <= 126` deserialize
        // check individually, but together they exceed the 63-byte storage buffer.
        // `put_chunk` must reject such a chunk gracefully instead of panicking.
        use nimiq_primitives::trie::trie_node::TrieNodeChild;

        let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
        let trie = MerkleRadixTrie::new_incomplete(&env, TestTrie);
        let mut raw_txn = env.write_transaction();
        let mut txn: WriteTransactionProxy = (&mut raw_txn).into();
        assert!(!trie.is_complete(&txn));

        let parent_key: KeyNibbles = "a".repeat(40).parse().unwrap();
        let oversized_suffix: KeyNibbles = "b".repeat(90).parse().unwrap();

        let mut proof_node: TrieProofNode = TrieNode::new_empty(parent_key.clone()).into();
        // 40 + 90 = 130 nibbles > MAX_BYTES * 2 = 126 — would overflow `Add`.
        proof_node.children[0] = Some(TrieNodeChild {
            suffix: oversized_suffix,
            hash: Blake2bHash::default(),
        });

        let proof_root = TrieNode::new_root();
        let expected_hash = proof_root.hash_assert();
        let proof = TrieProof::new(vec![proof_node, proof_root.into()], Default::default());

        let malicious_chunk =
            TrieChunk::new(None, vec![TrieItem::new(parent_key, vec![0x42])], proof);

        // The exact error doesn't matter — what matters is that this returns instead
        // of panicking inside `KeyNibbles::Add`.
        assert!(trie
            .put_chunk(&mut txn, KeyNibbles::ROOT, malicious_chunk, expected_hash)
            .is_err());
    }

    #[test]
    fn apply_diff_with_root_key_returns_error_instead_of_panicking() {
        // A ROOT-keyed entry in a TrieDiff used to reach
        // `update_within_missing_part_raw` and panic on the descent loop's
        // `assert_ne!(&node.key, key)`; `apply_diff` must return an error.
        let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
        let trie = MerkleRadixTrie::new_incomplete(&env, TestTrie);
        let mut raw_txn = env.write_transaction();
        let mut txn: WriteTransactionProxy = (&mut raw_txn).into();
        assert!(!trie.is_complete(&txn));

        let mut diff_map = BTreeMap::new();
        diff_map.insert(KeyNibbles::ROOT, Some(vec![1, 2, 3]));
        match trie.apply_diff(&mut txn, TrieDiff(diff_map)) {
            Err(MerkleRadixTrieError::InvalidChunk(_)) => {}
            other => panic!("expected InvalidChunk(_), got {:?}", other),
        }

        let mut diff_map = BTreeMap::new();
        diff_map.insert(KeyNibbles::ROOT, None);
        match trie.apply_diff(&mut txn, TrieDiff(diff_map)) {
            Err(MerkleRadixTrieError::InvalidChunk(_)) => {}
            other => panic!("expected InvalidChunk(_), got {:?}", other),
        }
    }

    #[test]
    fn apply_diff_root_key_in_complete_part_returns_error_instead_of_panicking() {
        // A ROOT-keyed diff entry on a PARTIALLY-synced incomplete trie
        // (missing_range.start != ROOT) routes to the COMPLETE branch -> put_raw ->
        // put_value().unwrap() -> RootCantHaveValue panic. The only guard there was a
        // release-stripped debug_assert!. The other root-key test only covers a
        // FRESH incomplete trie (start == ROOT -> already-guarded missing branch).
        let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
        let original = MerkleRadixTrie::new(&env, TestTrie);
        let trie = MerkleRadixTrie::new_incomplete(&env, TestTrieCopy);
        let mut raw_txn = env.write_transaction();
        let mut txn: WriteTransactionProxy = (&mut raw_txn).into();

        let key_1: KeyNibbles = "413f22".parse().unwrap();
        let key_2: KeyNibbles = "413".parse().unwrap();
        let key_3: KeyNibbles = "413f227fa".parse().unwrap();
        let key_4: KeyNibbles = "413b391".parse().unwrap();
        original.put(&mut txn, &key_1, 80085).unwrap();
        original.put(&mut txn, &key_2, 999).unwrap();
        original.put(&mut txn, &key_3, 1337).unwrap();
        original.put(&mut txn, &key_4, 6969).unwrap();
        original.update_root(&mut txn).unwrap();
        let hash = original.root_hash_assert(&txn);

        // Partially sync: a single limit-0 chunk from ROOT advances the missing
        // range's start past ROOT while leaving the trie incomplete.
        let chunk = original.get_chunk_with_proof(&txn, KeyNibbles::ROOT.., 0);
        trie.put_chunk(&mut txn, KeyNibbles::ROOT, chunk, hash)
            .unwrap();
        assert!(!trie.is_complete(&txn));

        let missing_range = trie.get_missing_range(&txn);
        let start = missing_range.as_ref().unwrap().start.clone();
        assert!(
            !start.is_empty(),
            "setup: missing-range start must be non-empty"
        );
        assert!(
            trie.is_within_complete_part(&KeyNibbles::ROOT, &missing_range),
            "setup: ROOT must route to the complete (put_raw) branch",
        );

        let mut diff_map = BTreeMap::new();
        diff_map.insert(KeyNibbles::ROOT, Some(vec![1u8, 2, 3]));
        match trie.apply_diff(&mut txn, TrieDiff(diff_map)) {
            Err(MerkleRadixTrieError::InvalidChunk(_)) => {}
            other => panic!("expected InvalidChunk(_), got {:?}", other),
        }
    }

    #[test]
    fn partial_tree_put_chunks_manual() {
        let key_1 = "413f22".parse().unwrap();
        let key_2 = "413".parse().unwrap();
        let key_3 = "413f227fa".parse().unwrap();
        let key_4 = "413b391".parse().unwrap();
        let key_5 = "412324".parse().unwrap();

        let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
        let original = MerkleRadixTrie::new(&env, TestTrie);
        let trie = MerkleRadixTrie::new_incomplete(&env, TestTrieCopy);
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

        let env = MdbxDatabase::new_volatile(Default::default()).unwrap();

        let original = MerkleRadixTrie::new(&env, TestTrie);
        let copy = MerkleRadixTrie::new_incomplete(&env, TestTrieCopy);
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
        for chunk_size in 1..5 {
            copy.reinitialize_as_incomplete(&mut txn);
            let mut next_start = KeyNibbles::ROOT;
            for _ in 0..10 {
                let start = next_start;
                let chunk = original.get_chunk_with_proof(&txn, start.clone().., chunk_size);

                copy.put_chunk(&mut txn, start, chunk.clone(), hash.clone())
                    .unwrap();

                if copy.is_complete(&txn) {
                    break;
                }
                next_start = chunk.end_key.unwrap();
            }
            assert!(copy.is_complete(&txn));
            assert_eq!(copy.count_nodes(&txn), (0, 2, 2));

            assert_eq!(copy.get(&txn, &key_1).expect("complete trie"), Some(80085));
            assert_eq!(copy.get(&txn, &key_2).expect("complete trie"), Some(999));
            assert_eq!(copy.get(&txn, &key_3).expect("complete trie"), Some(1337));
            assert_eq!(copy.get(&txn, &key_4).expect("complete trie"), Some(6969));
            assert_eq!(copy.get(&txn, &key_5).expect("complete trie"), None::<i32>);
        }
    }

    #[test]
    fn partial_tree_put_chunks_of_different_sizes_2() {
        let key_1 = "1a".parse().unwrap();
        let key_2 = "1b1a".parse().unwrap();
        let key_3 = "1b1b".parse().unwrap();
        let key_4 = "1c".parse().unwrap();
        let key_5 = "81".parse().unwrap();

        let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
        let original = MerkleRadixTrie::new(&env, TestTrie);
        let copy = MerkleRadixTrie::new_incomplete(&env, TestTrieCopy);
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
        for chunk_size in 1..5 {
            copy.reinitialize_as_incomplete(&mut txn);
            let mut next_start = KeyNibbles::ROOT;
            for _ in 0..10 {
                let start = next_start;
                let chunk = original.get_chunk_with_proof(&txn, start.clone().., chunk_size);

                copy.put_chunk(&mut txn, start, chunk.clone(), hash.clone())
                    .unwrap();

                if copy.is_complete(&txn) {
                    break;
                }
                next_start = chunk.end_key.unwrap();
            }
            assert!(copy.is_complete(&txn));
            assert_eq!(copy.count_nodes(&txn), (2, 0, 5));

            assert_eq!(copy.get(&txn, &key_1).expect("complete trie"), Some(80085));
            assert_eq!(copy.get(&txn, &key_2).expect("complete trie"), Some(999));
            assert_eq!(copy.get(&txn, &key_3).expect("complete trie"), Some(1337));
            assert_eq!(copy.get(&txn, &key_4).expect("complete trie"), Some(6969));
            assert_eq!(copy.get(&txn, &key_5).expect("complete trie"), Some(2969));
        }
    }

    #[test]
    fn partial_tree_remove_chunks_of_different_sizes() {
        let key_1 = "413f22".parse().unwrap();
        let key_2 = "413".parse().unwrap();
        let key_3 = "413f227fa".parse().unwrap();
        let key_4 = "413b391".parse().unwrap();
        let key_5 = "412324".parse().unwrap();

        let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
        let original = MerkleRadixTrie::new(&env, TestTrie);

        let init_trie = |env: &MdbxDatabase| -> MerkleRadixTrie<TestTrieCopy> {
            env.create_regular_table(&TestTrieCopy);
            let mut raw_txn = env.write_transaction();
            raw_txn.clear_table(&TestTrieCopy);
            raw_txn.commit();
            let trie = MerkleRadixTrie::new(env, TestTrieCopy);
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
            trie
        };

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
        raw_txn.commit();

        for chunk_size in 1..5 {
            let trie = init_trie(&env);

            let mut raw_txn = env.write_transaction();
            let mut txn: WriteTransactionProxy = (&mut raw_txn).into();

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

        let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
        let original = MerkleRadixTrie::new(&env, TestTrie);

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

        let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
        let original = MerkleRadixTrie::new(&env, TestTrie);

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
        let key_1: KeyNibbles = "413f22".parse().unwrap();

        let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
        let original = MerkleRadixTrie::new(&env, TestTrie);

        let mut raw_txn = env.write_transaction();
        let mut txn: WriteTransactionProxy = (&mut raw_txn).into();

        original.remove_chunk(&mut txn, key_1.clone()).unwrap();
        assert_eq!(original.get_missing_range(&txn), Some(key_1..));
        assert_eq!(original.count_nodes(&txn), (0, 0, 0));
    }

    #[test]
    fn remove_empty_chunk() {
        let key_1 = "413f22".parse().unwrap();
        let key_2 = "413".parse().unwrap();
        let key_3 = "413f227fa".parse().unwrap();
        let key_4 = "413b391".parse().unwrap();
        let key_5: KeyNibbles = "415324".parse().unwrap();

        let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
        let original = MerkleRadixTrie::new(&env, TestTrie);

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

        // Nothing is stored from `key_5` on, but the trie is still missing that part afterwards
        original.remove_chunk(&mut txn, key_5.clone()).unwrap();
        assert_eq!(original.count_nodes(&txn), (0, 2, 2));
        assert_eq!(original.get_missing_range(&txn), Some(key_5.clone()..));

        // `key_1` keeps `key_3` as a stump child, so it stays a hybrid node
        let hash = original.root_hash_assert(&txn);
        original.remove_chunk(&mut txn, key_3.clone()).unwrap();
        assert_eq!(original.count_nodes(&txn), (0, 2, 1));
        assert_eq!(original.get_missing_range(&txn), Some(key_3.clone()..));
        assert_eq!(original.root_hash(&txn), Some(hash));

        // The trie is already missing everything from `key_3` on
        original.remove_chunk(&mut txn, key_5).unwrap();
        assert_eq!(original.count_nodes(&txn), (0, 2, 1));
        assert_eq!(original.get_missing_range(&txn), Some(key_3..));
    }

    #[test]
    fn remove_and_put_chunk() {
        let key_1 = "413f22".parse().unwrap();
        let key_2 = "413".parse().unwrap();
        let key_3 = "413f227fa".parse().unwrap();
        let key_4 = "413b391".parse().unwrap();

        let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
        let original = MerkleRadixTrie::new(&env, TestTrie);

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

    fn fill(txn: &mut WriteTransactionProxy, full: &MerkleRadixTrie<TestTrie>, keys: &[&str]) {
        for (i, key) in keys.iter().enumerate() {
            full.put_raw(txn, &key.parse().unwrap(), vec![i as u8], &None);
        }
        full.update_root(txn).unwrap();
    }

    /// Applies an honest chunk of `size` items to the partial trie.
    fn push_chunk(
        txn: &mut WriteTransactionProxy,
        full: &MerkleRadixTrie<TestTrie>,
        partial: &MerkleRadixTrie<TestTrieCopy>,
        size: usize,
    ) {
        let start = partial.get_missing_range(txn).unwrap().start;
        let chunk = full.get_chunk_with_proof(txn, start.clone().., size);
        let hash = full.root_hash_assert(txn);
        assert_eq!(
            partial.put_chunk(txn, start, chunk, hash),
            Ok(TrieChunkPushResult::Applied)
        );
    }

    /// Applies a diff to both tries and checks that the partial trie doesn't report a wrong hash.
    fn apply_to_both(
        txn: &mut WriteTransactionProxy,
        full: &MerkleRadixTrie<TestTrie>,
        partial: &MerkleRadixTrie<TestTrieCopy>,
        diff: &[(&str, Option<u8>)],
    ) -> (RevertTrieDiff, RevertTrieDiff) {
        let diff = TrieDiff(
            diff.iter()
                .map(|(key, value)| (key.parse().unwrap(), value.map(|v| vec![v])))
                .collect(),
        );
        let full_revert = full.apply_diff(txn, diff.clone()).unwrap();
        full.update_root(txn).unwrap();
        let partial_revert = partial.apply_diff(txn, diff).unwrap();
        partial.update_root(txn).ok();
        assert_consistent(txn, full, partial);
        (full_revert, partial_revert)
    }

    fn assert_consistent(
        txn: &MdbxReadTransaction,
        full: &MerkleRadixTrie<TestTrie>,
        partial: &MerkleRadixTrie<TestTrieCopy>,
    ) {
        if let Some(hash) = partial.root_hash(txn) {
            assert_eq!(hash, full.root_hash_assert(txn));
        }
        partial.count_nodes(txn);
    }

    /// Syncs the rest of the partial trie and checks that it ends up equal to the full trie.
    fn finish_sync(
        txn: &mut WriteTransactionProxy,
        full: &MerkleRadixTrie<TestTrie>,
        partial: &MerkleRadixTrie<TestTrieCopy>,
    ) {
        while !partial.is_complete(txn) {
            push_chunk(txn, full, partial, 1);
        }
        assert_eq!(partial.root_hash_assert(txn), full.root_hash_assert(txn));
        assert_eq!(partial.count_nodes(txn), full.count_nodes(txn));
    }

    #[test]
    fn removing_the_last_complete_child_merges_the_branch_into_a_stump() {
        let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
        let full = MerkleRadixTrie::new(&env, TestTrie);
        let partial = MerkleRadixTrie::new_incomplete(&env, TestTrieCopy);
        let mut raw_txn = env.write_transaction();
        let mut txn: WriteTransactionProxy = (&mut raw_txn).into();
        fill(&mut txn, &full, &["a1", "a2", "b"]);

        // The chunk ends between the only two children of branch `a`
        push_chunk(&mut txn, &full, &partial, 1);
        assert_eq!(
            partial.get_missing_range(&txn),
            Some("a2".parse().unwrap()..)
        );

        // Branch `a` is left with a single stump child
        apply_to_both(&mut txn, &full, &partial, &[("a1", None)]);
        finish_sync(&mut txn, &full, &partial);
    }

    #[test]
    fn removing_a_hybrid_value_merges_the_node_into_a_stump() {
        let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
        let full = MerkleRadixTrie::new(&env, TestTrie);
        let partial = MerkleRadixTrie::new_incomplete(&env, TestTrieCopy);
        let mut raw_txn = env.write_transaction();
        let mut txn: WriteTransactionProxy = (&mut raw_txn).into();
        fill(&mut txn, &full, &["a", "a1", "b"]);

        push_chunk(&mut txn, &full, &partial, 1);
        assert_eq!(
            partial.get_missing_range(&txn),
            Some("a1".parse().unwrap()..)
        );

        // Hybrid `a` becomes a branch with a single stump child
        apply_to_both(&mut txn, &full, &partial, &[("a", None)]);
        finish_sync(&mut txn, &full, &partial);
    }

    #[test]
    fn new_key_in_missing_part_splits_a_complete_node() {
        let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
        let full = MerkleRadixTrie::new(&env, TestTrie);
        let partial = MerkleRadixTrie::new_incomplete(&env, TestTrieCopy);
        let mut raw_txn = env.write_transaction();
        let mut txn: WriteTransactionProxy = (&mut raw_txn).into();
        fill(&mut txn, &full, &["5a1", "5a2", "6"]);

        push_chunk(&mut txn, &full, &partial, 1);
        assert_eq!(
            partial.get_missing_range(&txn),
            Some("5a2".parse().unwrap()..)
        );

        // `5f` is in the missing part, but diverges from the complete branch `5a`
        apply_to_both(&mut txn, &full, &partial, &[("5f", Some(1))]);
        finish_sync(&mut txn, &full, &partial);
    }

    #[test]
    fn update_in_missing_part_keeps_the_stump() {
        let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
        let full = MerkleRadixTrie::new(&env, TestTrie);
        let partial = MerkleRadixTrie::new_incomplete(&env, TestTrieCopy);
        let mut raw_txn = env.write_transaction();
        let mut txn: WriteTransactionProxy = (&mut raw_txn).into();
        fill(&mut txn, &full, &["a1", "b12", "b13"]);

        push_chunk(&mut txn, &full, &partial, 1);
        assert_eq!(
            partial.get_missing_range(&txn),
            Some("b".parse().unwrap()..)
        );

        // `b2` is next to the stump for branch `b1`, which must keep pointing to `b1`
        apply_to_both(&mut txn, &full, &partial, &[("b2", Some(1))]);
        let root = partial.get_root(&txn).unwrap();
        let stump = root.child(&"b".parse().unwrap()).unwrap();
        assert_eq!(stump.suffix, "b1".parse().unwrap());
        assert!(!stump.has_hash());

        finish_sync(&mut txn, &full, &partial);
    }

    #[test]
    fn new_key_below_a_leaf_in_missing_part_is_counted() {
        let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
        let full = MerkleRadixTrie::new(&env, TestTrie);
        let partial = MerkleRadixTrie::new_incomplete(&env, TestTrieCopy);
        let mut raw_txn = env.write_transaction();
        let mut txn: WriteTransactionProxy = (&mut raw_txn).into();
        fill(&mut txn, &full, &["0", "22"]);

        // A valid chunk that ends right below leaf `0`
        let key_0: KeyNibbles = "0".parse().unwrap();
        let proof = full.get_proof(&txn, vec![&key_0]).unwrap();
        let chunk = TrieChunk::new(
            Some("01".parse().unwrap()),
            vec![TrieItem::new(key_0, vec![0])],
            proof,
        );
        let hash = full.root_hash_assert(&txn);
        assert_eq!(
            partial.put_chunk(&mut txn, KeyNibbles::ROOT, chunk, hash),
            Ok(TrieChunkPushResult::Applied)
        );

        // Leaf `0` gets a stump child and becomes a hybrid node
        apply_to_both(&mut txn, &full, &partial, &[("02", Some(1))]);
        assert_eq!(partial.count_nodes(&txn), (0, 1, 0));
        finish_sync(&mut txn, &full, &partial);
    }

    #[test]
    fn removing_a_chunk_keeps_the_nodes_above_its_start() {
        let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
        let full = MerkleRadixTrie::new(&env, TestTrie);
        let partial = MerkleRadixTrie::new_incomplete(&env, TestTrieCopy);
        let mut raw_txn = env.write_transaction();
        let mut txn: WriteTransactionProxy = (&mut raw_txn).into();
        fill(&mut txn, &full, &["12", "3", "3000", "302", "311"]);

        push_chunk(&mut txn, &full, &partial, 3);
        let start = partial.get_missing_range(&txn).unwrap().start;
        assert_eq!(start, "302".parse().unwrap());

        // A block removes everything below `3` that is before the missing part, and its chunk
        // completes the trie
        let (full_revert, partial_revert) =
            apply_to_both(&mut txn, &full, &partial, &[("3", None), ("3000", None)]);
        push_chunk(&mut txn, &full, &partial, 3);
        assert!(partial.is_complete(&txn));

        // Reverting the block needs node `3` for the missing part again
        partial.remove_chunk(&mut txn, start).unwrap();
        partial.revert_diff(&mut txn, partial_revert).unwrap();
        partial.update_root(&mut txn).ok();
        full.revert_diff(&mut txn, full_revert).unwrap();
        full.update_root(&mut txn).unwrap();
        assert_eq!(partial.root_hash(&txn), Some(full.root_hash_assert(&txn)));
        assert_consistent(&txn, &full, &partial);

        finish_sync(&mut txn, &full, &partial);
    }

    #[test]
    fn reverting_a_block_after_its_chunk_only_marks_stumps() {
        let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
        let full = MerkleRadixTrie::new(&env, TestTrie);
        let partial = MerkleRadixTrie::new_incomplete(&env, TestTrieCopy);
        let mut raw_txn = env.write_transaction();
        let mut txn: WriteTransactionProxy = (&mut raw_txn).into();
        fill(&mut txn, &full, &["0", "10", "12", "2"]);

        // The partial trie is still empty, so the block adds a stump for `12`
        let (full_revert, partial_revert) =
            apply_to_both(&mut txn, &full, &partial, &[("12", None)]);
        // The block's chunk replaces it by a stump for all of branch `1`
        push_chunk(&mut txn, &full, &partial, 1);
        assert_eq!(
            partial.get_missing_range(&txn),
            Some("1".parse().unwrap()..)
        );

        // Reverting the block must not remove the stump for branch `1`
        partial.remove_chunk(&mut txn, KeyNibbles::ROOT).unwrap();
        partial.revert_diff(&mut txn, partial_revert).unwrap();
        partial.update_root(&mut txn).ok();
        full.revert_diff(&mut txn, full_revert).unwrap();
        full.update_root(&mut txn).unwrap();
        assert_consistent(&txn, &full, &partial);

        finish_sync(&mut txn, &full, &partial);
    }

    #[test]
    fn reverting_a_block_whose_chunk_completed_the_trie() {
        let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
        let full = MerkleRadixTrie::new(&env, TestTrie);
        let partial = MerkleRadixTrie::new_incomplete(&env, TestTrieCopy);
        let mut raw_txn = env.write_transaction();
        let mut txn: WriteTransactionProxy = (&mut raw_txn).into();
        fill(&mut txn, &full, &["0", "1"]);

        push_chunk(&mut txn, &full, &partial, 1);
        let start = partial.get_missing_range(&txn).unwrap().start;
        assert_eq!(start, "1".parse().unwrap());

        // The block removes everything in the missing part, so its chunk has no items
        let (full_revert, partial_revert) =
            apply_to_both(&mut txn, &full, &partial, &[("1", None)]);
        push_chunk(&mut txn, &full, &partial, 1);
        assert!(partial.is_complete(&txn));

        // Reverting the block makes the part from the chunk's start missing again
        partial.remove_chunk(&mut txn, start.clone()).unwrap();
        assert_eq!(partial.get_missing_range(&txn), Some(start..));
        partial.revert_diff(&mut txn, partial_revert).unwrap();
        partial.update_root(&mut txn).ok();
        full.revert_diff(&mut txn, full_revert).unwrap();
        full.update_root(&mut txn).unwrap();
        assert_consistent(&txn, &full, &partial);

        finish_sync(&mut txn, &full, &partial);
    }

    #[test]
    fn clearing_stumps_removes_branches_that_no_longer_exist() {
        let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
        let full = MerkleRadixTrie::new(&env, TestTrie);
        let partial = MerkleRadixTrie::new_incomplete(&env, TestTrieCopy);
        let mut raw_txn = env.write_transaction();
        let mut txn: WriteTransactionProxy = (&mut raw_txn).into();
        fill(&mut txn, &full, &["1", "21", "23", "24"]);

        push_chunk(&mut txn, &full, &partial, 2);
        assert_eq!(
            partial.get_missing_range(&txn),
            Some("23".parse().unwrap()..)
        );

        // Branch `2` is left with only `23`, so it no longer exists in the full trie. The
        // partial trie still has it with two stumps.
        apply_to_both(&mut txn, &full, &partial, &[("21", None), ("24", None)]);

        // The next chunk must not keep branch `2` with `23` as its only child
        finish_sync(&mut txn, &full, &partial);
    }

    #[test]
    fn clearing_stumps_merges_branches_left_with_one_complete_child() {
        let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
        let full = MerkleRadixTrie::new(&env, TestTrie);
        let partial = MerkleRadixTrie::new_incomplete(&env, TestTrieCopy);
        let mut raw_txn = env.write_transaction();
        let mut txn: WriteTransactionProxy = (&mut raw_txn).into();
        fill(&mut txn, &full, &["1", "21", "23", "3"]);

        push_chunk(&mut txn, &full, &partial, 2);
        assert_eq!(
            partial.get_missing_range(&txn),
            Some("23".parse().unwrap()..)
        );

        // Branch `2` is left with only `21`, so it no longer exists in the full trie. The
        // partial trie still has it with a stump for `23`.
        apply_to_both(&mut txn, &full, &partial, &[("23", None)]);

        // The next chunk must merge branch `2` into `21`
        finish_sync(&mut txn, &full, &partial);
    }

    /// Deterministic xorshift generator for the state sync simulation.
    struct SimRng(u64);

    impl SimRng {
        fn below(&mut self, n: u64) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0 % n
        }

        /// Short keys over a small alphabet, so that keys share prefixes, become hybrids and
        /// split each other's branches.
        fn key(&mut self) -> KeyNibbles {
            let len = 1 + self.below(4) as usize;
            (0..len)
                .map(|_| char::from_digit(self.below(4) as u32, 16).unwrap())
                .collect::<String>()
                .parse()
                .unwrap()
        }

        /// A diff that creates, updates and prunes entries of `state`.
        fn diff(&mut self, state: &BTreeMap<KeyNibbles, Vec<u8>>) -> TrieDiff {
            let mut diff = BTreeMap::new();
            for _ in 0..=self.below(3) {
                if !state.is_empty() && self.below(3) == 0 {
                    let idx = self.below(state.len() as u64) as usize;
                    let key = state.keys().nth(idx).unwrap().clone();
                    diff.entry(key).or_insert(None);
                } else {
                    let key = self.key();
                    let value = vec![self.below(256) as u8];
                    if state.get(&key) != Some(&value) {
                        diff.entry(key).or_insert(Some(value));
                    }
                }
            }
            TrieDiff(diff)
        }
    }

    struct SimBlock {
        full_revert: RevertTrieDiff,
        partial_revert: RevertTrieDiff,
        prev_missing_start: Option<KeyNibbles>,
        prev_state: BTreeMap<KeyNibbles, Vec<u8>>,
    }

    /// Simulates a node that state-syncs a trie while blocks are applied and reverted, and
    /// checks it against a complete trie that follows the same blocks.
    fn simulate_state_sync(seed: u64) {
        let mut rng = SimRng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
        let full = MerkleRadixTrie::new(&env, TestTrie);
        let partial = MerkleRadixTrie::new_incomplete(&env, TestTrieCopy);
        let mut raw_txn = env.write_transaction();
        let mut txn: WriteTransactionProxy = (&mut raw_txn).into();

        let mut state = BTreeMap::new();
        for _ in 0..(3 + rng.below(12)) {
            let key = rng.key();
            let value = vec![rng.below(256) as u8];
            full.put_raw(&mut txn, &key, value.clone(), &None);
            state.insert(key, value);
        }
        full.update_root(&mut txn).unwrap();

        let mut blocks: Vec<SimBlock> = vec![];
        let mut step = 0;
        loop {
            let ctx = format!("seed {seed}, step {step}");
            let action = if step < 80 { rng.below(5) } else { 0 };
            match action {
                // Apply a chunk of the current state
                0 | 1 => {
                    let Some(missing) = partial.get_missing_range(&txn) else {
                        if step >= 80 {
                            break;
                        }
                        step += 1;
                        continue;
                    };
                    let chunk = full.get_chunk_with_proof(
                        &txn,
                        missing.start.clone()..,
                        1 + rng.below(3) as usize,
                    );
                    let hash = full.root_hash_assert(&txn);
                    assert_eq!(
                        partial.put_chunk(&mut txn, missing.start.clone(), chunk, hash),
                        Ok(TrieChunkPushResult::Applied),
                        "{ctx}: chunk from {}",
                        missing.start,
                    );
                    if let Some(block) = blocks.last_mut() {
                        block.prev_missing_start.get_or_insert(missing.start);
                    }
                }
                // Apply a block
                2 | 3 => {
                    let diff = rng.diff(&state);
                    let prev_state = state.clone();
                    for (key, value) in &diff.0 {
                        match value {
                            Some(value) => state.insert(key.clone(), value.clone()),
                            None => state.remove(key),
                        };
                    }
                    let full_revert = full.apply_diff(&mut txn, diff.clone()).unwrap();
                    full.update_root(&mut txn).unwrap();
                    let partial_revert = partial
                        .apply_diff(&mut txn, diff.clone())
                        .unwrap_or_else(|e| panic!("{ctx}: apply {diff:?}: {e}"));
                    partial.update_root(&mut txn).ok();
                    blocks.push(SimBlock {
                        full_revert,
                        partial_revert,
                        prev_missing_start: None,
                        prev_state,
                    });
                }
                // Revert the latest block, including the chunks applied with it
                _ => {
                    if let Some(block) = blocks.pop() {
                        if let Some(start) = block.prev_missing_start {
                            partial
                                .remove_chunk(&mut txn, start)
                                .unwrap_or_else(|e| panic!("{ctx}: remove chunk: {e}"));
                        }
                        partial
                            .revert_diff(&mut txn, block.partial_revert)
                            .unwrap_or_else(|e| panic!("{ctx}: revert: {e}"));
                        partial.update_root(&mut txn).ok();
                        full.revert_diff(&mut txn, block.full_revert).unwrap();
                        full.update_root(&mut txn).unwrap();
                        state = block.prev_state;
                    }
                }
            }

            // A partial trie must never report a root hash that differs from the full trie
            if let Some(hash) = partial.root_hash(&txn) {
                assert_eq!(hash, full.root_hash_assert(&txn), "{ctx}: root hash");
            }
            // Checks the node counters against the actual nodes
            partial.count_nodes(&txn);
            step += 1;
        }

        assert_eq!(
            partial.root_hash_assert(&txn),
            full.root_hash_assert(&txn),
            "seed {seed}: final root hash"
        );
        assert_eq!(
            partial.count_nodes(&txn),
            full.count_nodes(&txn),
            "seed {seed}: final node counts"
        );
    }

    #[test]
    fn partial_trie_follows_blocks_during_state_sync() {
        for seed in 1..=20 {
            simulate_state_sync(seed);
        }
    }
}
