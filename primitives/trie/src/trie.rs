use std::marker::PhantomData;
use std::mem;
use std::sync::atomic;
use std::sync::atomic::AtomicU64;

use log::error;

use beserial::{Deserialize, Serialize};
use nimiq_database::{Database, Environment, Transaction, WriteTransaction};
use nimiq_hash::{Blake2bHash, Hash};

use crate::key_nibbles::KeyNibbles;
use crate::trie_node::TrieNode;
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
#[derive(Debug)]
pub struct MerkleRadixTrie<A: Serialize + Deserialize + Clone> {
    db: Database,
    num_branches: AtomicU64,
    num_leaves: AtomicU64,
    _value: PhantomData<A>,
}

impl<A: Serialize + Deserialize + Clone> MerkleRadixTrie<A> {
    /// Start a new Merkle Radix Trie with the given Environment and the given name.
    pub fn new(env: Environment, name: &str) -> Self {
        let db = env.open_database(name.to_string());

        let tree = MerkleRadixTrie {
            db,
            num_branches: AtomicU64::new(1),
            num_leaves: AtomicU64::new(0),
            _value: PhantomData,
        };

        let mut txn = WriteTransaction::new(&env);
        if tree.get_root(&txn).is_none() {
            txn.put_reserve(&tree.db, &KeyNibbles::ROOT, &TrieNode::new_root());
        }
        txn.commit();

        tree
    }

    /// Returns the root hash of the Merkle Radix Trie.
    pub fn root_hash(&self, txn: &Transaction) -> Blake2bHash {
        self.get_root(txn).unwrap().hash()
    }

    /// Returns the number of branch nodes in the Merkle Radix Trie.
    pub fn num_branches(&self) -> u64 {
        self.num_branches.load(atomic::Ordering::Acquire)
    }

    /// Returns the number of leaf nodes in the Merkle Radix Trie.
    pub fn num_leaves(&self) -> u64 {
        self.num_leaves.load(atomic::Ordering::Acquire)
    }

    #[cfg(test)]
    fn count_branches_and_leaves(&self, txn: &Transaction) -> (u64, u64) {
        let mut num_branches = 0;
        let mut num_leaves = 0;

        let mut stack = vec![self
            .get_root(txn)
            .expect("The Merkle Radix Trie didn't have a root node!")];

        while let Some(item) = stack.pop() {
            if let Ok(children) = item.children() {
                for child in children.iter().flatten().rev() {
                    let combined = item.key() + &child.suffix;

                    stack.push(txn.get(&self.db, &combined)
                        .expect("Failed to find the child of a Merkle Radix Trie node. The database must be corrupt!"));
                }
                num_branches += 1;
            } else {
                num_leaves += 1;
            }
        }

        assert_eq!(self.num_branches(), num_branches);
        assert_eq!(self.num_leaves(), num_leaves);

        (num_branches, num_leaves)
    }

    /// Get the value at the given key. If there's no leaf or hybrid node at the given key then it
    /// returns None.
    pub fn get(&self, txn: &Transaction, key: &KeyNibbles) -> Option<A> {
        let node: TrieNode = txn.get(&self.db, key)?;
        node.value().ok()
    }

    /// Returns a chunk of the Merkle Radix Trie that starts at the key `start` (which might or not
    /// be a part of the trie, if it is then it will be part of the chunk) and contains at most
    /// `size` leaf nodes.
    pub fn get_chunk(&self, txn: &Transaction, start: &KeyNibbles, size: usize) -> Vec<A> {
        let chunk = self.get_trie_chunk(txn, start, size);

        chunk
            .into_iter()
            .map(|node| node.value().unwrap())
            .collect()
    }

    /// Insert a value into the Merkle Radix Trie at the given key. If the key already exists then
    /// it will overwrite it. You can't use this function to check the existence of a given key.
    pub fn put(&self, txn: &mut WriteTransaction, key: &KeyNibbles, value: A) {
        // Start by getting the root node.
        let mut cur_node = self
            .get_root(txn)
            .expect("Merkle Radix Trie must have a root node!");

        // And initialize the root path.
        let mut root_path: Vec<TrieNode> = vec![];
        let added_branches;
        let added_leaves;

        // Go down the trie until you find where to put the new value.
        loop {
            // If the current node key is no longer a prefix for the given key then we need to
            // split the node.
            if !cur_node.key().is_prefix_of(key) {
                // Create and store the new node.
                let new_node = TrieNode::new_leaf(key.clone(), value);
                txn.put_reserve(&self.db, key, &new_node);

                // Create and store the new parent node.
                let new_parent = TrieNode::new_branch(cur_node.key().common_prefix(key))
                    .put_child(cur_node.key(), Blake2bHash::default())
                    .unwrap()
                    .put_child(new_node.key(), new_node.hash())
                    .unwrap();
                txn.put_reserve(&self.db, new_parent.key(), &new_parent);

                // Push the parent node into the root path.
                root_path.push(new_parent);
                added_branches = 1;
                added_leaves = 1;

                break;
            }

            // If the current node key is equal to the given key, we have found an existing node
            // with the given key. Update the value.
            if cur_node.key() == key {
                // Update the node and store it.
                // TODO This unwrap() will fail when attempting to store a value at the root node
                cur_node = cur_node.put_value(value).unwrap();
                txn.put_reserve(&self.db, key, &cur_node);

                // Push the node into the root path.
                root_path.push(cur_node);
                added_branches = 0;
                added_leaves = 0;

                break;
            }

            // Try to find a child of the current node that matches our key.
            match cur_node.get_child_key(key) {
                // If no matching child exists, add a new child to the current node.
                Err(_) => {
                    // Create and store the new node.
                    let new_node = TrieNode::new_leaf(key.clone(), value);
                    txn.put_reserve(&self.db, key, &new_node);

                    // Update the parent node and store it.
                    cur_node = cur_node.put_child(new_node.key(), new_node.hash()).unwrap();
                    txn.put_reserve(&self.db, cur_node.key(), &cur_node);

                    // Push the parent node into the root path.
                    root_path.push(cur_node);
                    added_branches = 0;
                    added_leaves = 1;

                    break;
                }
                // If there's a child, then we update the current node and the root path, and
                // continue down the trie.
                Ok(child_key) => {
                    root_path.push(cur_node);
                    cur_node = txn.get(&self.db, &child_key).unwrap();
                }
            }
        }

        // Update the keys and hashes of the nodes that we modified.
        self.update_keys(txn, root_path, added_branches, added_leaves);
    }

    /// Removes the value in the Merkle Radix Trie at the given key. If the key doesn't exist
    /// then this function just returns silently. You can't use this to check the existence of a
    /// given prefix.
    pub fn remove(&self, txn: &mut WriteTransaction, key: &KeyNibbles) {
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
            if !cur_node.key().is_prefix_of(key) {
                return;
            }

            // If the current node key is equal to our given key, we have found our node.
            // Update/remove the node.
            if cur_node.key() == key {
                assert!(
                    !cur_node.is_branch(),
                    "We can't remove a branch node! The node has key {}.",
                    key
                );

                // If a node remains after removing the value, update it, otherwise remove the
                // node from the database.
                if let Some(cur_node) = cur_node.remove_value() {
                    txn.put_reserve(&self.db, key, &cur_node);
                } else {
                    txn.remove(&self.db, key);
                }

                break;
            }

            // Try to find a child of the current node that matches our key.
            match cur_node.get_child_key(key) {
                // If no matching child exists, then the key doesn't exist and we stop here.
                Err(_) => {
                    return;
                }
                // If there's a child, then we update the current node and the root path, and
                // continue down the trie.
                Ok(child_key) => {
                    root_path.push(cur_node);
                    cur_node = txn.get(&self.db, &child_key).unwrap();
                }
            }
        }

        // Walk along the root path towards the root node, starting with the immediate predecessor
        // of the node with the given key, and update the nodes along the way.
        let mut child_key = key.clone();

        while let Some(mut parent_node) = root_path.pop() {
            // Remove the child from the parent node.
            parent_node = parent_node.remove_child(&child_key).unwrap();

            // Get the root address and the number of children of the node.
            let root_address = KeyNibbles::ROOT;
            let num_children = parent_node.iter_children().count();

            // If the node has only a single child (and it isn't the root node), merge it with the
            // child.
            if num_children == 1 && parent_node.key() != &root_address {
                // Remove the node from the database.
                txn.remove(&self.db, parent_node.key());

                // Get the node's only child and add it to the root path.
                let only_child_key =
                    parent_node.key() + &parent_node.iter_children().next().unwrap().suffix.clone();

                let only_child = txn.get(&self.db, &only_child_key).unwrap();

                root_path.push(only_child);

                // Update the keys and hashes of the rest of the root path.
                self.update_keys(txn, root_path, -1, -1);

                return;
            }
            // If the node has any children, or it is the root node, we just store the
            // parent node in the database and the root path. Then we update the keys and hashes of
            // of the root path.
            else if num_children > 0 || parent_node.key() == &root_address {
                txn.put_reserve(&self.db, parent_node.key(), &parent_node);

                root_path.push(parent_node);

                self.update_keys(txn, root_path, 0, -1);

                return;
            }
            // Otherwise, our node must have no children and not be the root node. In this case we
            // need to remove it too, so we loop again.
            else {
                child_key = parent_node.key().clone();
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
    pub fn get_proof(&self, txn: &Transaction, mut keys: Vec<&KeyNibbles>) -> Option<TrieProof> {
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

        // Iterate over all the keys that we wish to prove.
        loop {
            // Go down the trie until we find a node with our key or we can't go any further.
            loop {
                // If the key does not match, the requested key is not part of this trie. In
                // this case, we can't produce a proof so we terminate now.
                if !pointer_node.key().is_prefix_of(cur_key) {
                    error!(
                        "Pointer node with key {} is not a prefix to the current node with key {}.",
                        pointer_node.key(),
                        cur_key
                    );
                    return None;
                }

                // If the key fully matches, we have found the requested node. We must check that
                // it is a leaf node, we don't want to prove branch nodes.
                if pointer_node.key() == cur_key {
                    if pointer_node.is_branch() {
                        error!(
                            "Pointer node with key {} is a branch node. We don't want to prove branch nodes.",
                            pointer_node.key(),
                        );
                        return None;
                    }

                    break;
                }

                // Otherwise, try to find a child of the pointer node that matches our key.
                match pointer_node.get_child_key(cur_key) {
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
                        let old_pointer_node =
                            mem::replace(&mut pointer_node, txn.get(&self.db, &child_key).unwrap());
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
            while !pointer_node.key().is_prefix_of(cur_key) {
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
    pub fn get_chunk_proof(
        &self,
        txn: &Transaction,
        start: &KeyNibbles,
        size: usize,
    ) -> Option<TrieProof> {
        let chunk = self.get_trie_chunk(txn, start, size);

        let chunk_keys = chunk.iter().map(|node| node.key()).collect();

        self.get_proof(txn, chunk_keys)
    }

    pub fn update_root(&self, txn: &mut WriteTransaction) {
        self.update_hashes(txn, &KeyNibbles::ROOT);
    }

    /// Returns the root node, if there is one.
    fn get_root(&self, txn: &Transaction) -> Option<TrieNode> {
        txn.get(&self.db, &KeyNibbles::ROOT)
    }

    /// Updates the keys for a chain of nodes and marks those nodes as dirty. It assumes that the
    /// path starts at the root node and that each consecutive node is a child of the previous node.
    fn update_keys(
        &self,
        txn: &mut WriteTransaction,
        mut root_path: Vec<TrieNode>,
        added_branches: i8,
        added_leaves: i8,
    ) {
        {
            // Save it directly if the root node doesn't have to be updated for other reasons.
            let only_root_needs_update = root_path.len() == 1;
            let root = root_path.first_mut().expect("Root path must not be empty");
            if added_branches != 0 || added_leaves != 0 {
                if let TrieNode::Root {
                    ref mut num_branches,
                    ref mut num_leaves,
                    ..
                } = root
                {
                    if added_branches >= 0 {
                        *num_branches += added_branches as u64;
                    } else {
                        *num_branches -= (-added_branches) as u64;
                    }
                    if added_leaves >= 0 {
                        *num_leaves += added_leaves as u64;
                    } else {
                        *num_leaves -= (-added_leaves) as u64;
                    }
                    self.num_branches
                        .store(*num_branches, atomic::Ordering::Release);
                    self.num_leaves
                        .store(*num_leaves, atomic::Ordering::Release);
                } else {
                    panic!("Root path must start with a root node");
                }
                if only_root_needs_update {
                    txn.put_reserve(&self.db, root.key(), root);
                    return;
                }
            }
        }

        // Get the first node in the path.
        let mut child_node = root_path.pop().expect("Root path must not be empty!");

        // Go up the root path until you get to the root.
        while let Some(mut parent_node) = root_path.pop() {
            // Update and store the parent node.
            parent_node = parent_node
                // Mark this node as dirty by storing the default hash.
                .put_child(child_node.key(), Blake2bHash::default())
                .unwrap();
            assert_eq!(
                root_path.is_empty(),
                matches!(parent_node, TrieNode::Root { .. })
            );
            txn.put_reserve(&self.db, parent_node.key(), &parent_node);

            child_node = parent_node;
        }
    }

    /// Updates the hashes of all dirty nodes in the subtree specified by `key`.
    fn update_hashes(&self, txn: &mut WriteTransaction, key: &KeyNibbles) -> Blake2bHash {
        let mut node: TrieNode = txn.get(&self.db, key).unwrap();
        if node.is_leaf() {
            return node.hash();
        }

        // Compute sub hashes if necessary.
        let default_hash = Blake2bHash::default();
        for mut child in node.iter_children_mut() {
            if child.hash == default_hash {
                // TODO This could be parallelized.
                child.hash = self.update_hashes(txn, &(key + &child.suffix));
            }
        }
        txn.put_reserve(&self.db, key, &node);
        node.hash()
    }

    /// Returns the nodes of the chunk of the Merkle Radix Trie that starts at the key `start` and
    /// has size `size`. This is used by the `get_chunk` and `get_chunk_proof` functions.
    fn get_trie_chunk(&self, txn: &Transaction, start: &KeyNibbles, size: usize) -> Vec<TrieNode> {
        let mut chunk = Vec::new();

        if size == 0_usize {
            return chunk;
        }

        let mut stack = vec![self
            .get_root(txn)
            .expect("The Merkle Radix Trie didn't have a root node!")];

        while let Some(item) = stack.pop() {
            if let Ok(children) = item.children() {
                for child in children.iter().flatten().rev() {
                    let combined = item.key() + &child.suffix;

                    if combined.is_prefix_of(start) || *start <= combined {
                        stack.push(txn.get(&self.db, &combined)
                            .expect("Failed to find the child of a Merkle Radix Trie node. The database must be corrupt!"));
                    }
                }
            }

            if item.has_value() {
                if start.len() < item.key().len() || start <= item.key() {
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

        assert_eq!(trie.count_branches_and_leaves(&txn), (1, 0));

        trie.put(&mut txn, &key_1, 80085);
        trie.put(&mut txn, &key_2, 999);
        trie.put(&mut txn, &key_3, 1337);

        assert_eq!(trie.count_branches_and_leaves(&txn), (3, 3));
        assert_eq!(trie.get(&txn, &key_1), Some(80085));
        assert_eq!(trie.get(&txn, &key_2), Some(999));
        assert_eq!(trie.get(&txn, &key_3), Some(1337));
        assert_eq!(trie.get(&txn, &key_4), None);

        trie.remove(&mut txn, &key_4);
        assert_eq!(trie.count_branches_and_leaves(&txn), (3, 3));
        assert_eq!(trie.get(&txn, &key_1), Some(80085));
        assert_eq!(trie.get(&txn, &key_2), Some(999));
        assert_eq!(trie.get(&txn, &key_3), Some(1337));

        trie.remove(&mut txn, &key_1);
        assert_eq!(trie.count_branches_and_leaves(&txn), (2, 2));
        assert_eq!(trie.get(&txn, &key_1), None);
        assert_eq!(trie.get(&txn, &key_2), Some(999));
        assert_eq!(trie.get(&txn, &key_3), Some(1337));

        trie.remove(&mut txn, &key_2);
        assert_eq!(trie.count_branches_and_leaves(&txn), (1, 1));
        assert_eq!(trie.get(&txn, &key_1), None);
        assert_eq!(trie.get(&txn, &key_2), None);
        assert_eq!(trie.get(&txn, &key_3), Some(1337));

        trie.remove(&mut txn, &key_3);
        assert_eq!(trie.count_branches_and_leaves(&txn), (1, 0));
        assert_eq!(trie.get(&txn, &key_1), None);
        assert_eq!(trie.get(&txn, &key_2), None);
        assert_eq!(trie.get(&txn, &key_3), None);
    }

    #[test]
    fn get_proof_works() {
        let key_1 = "cfb986f5a".parse().unwrap();
        let key_2 = "cfb986ab9".parse().unwrap();
        let key_3 = "cfb98e0f6".parse().unwrap();
        let key_4 = "cfb98e0f5".parse().unwrap();

        let env = nimiq_database::volatile::VolatileEnvironment::new(10).unwrap();
        let trie = MerkleRadixTrie::new(env.clone(), "database");
        let mut txn = WriteTransaction::new(&env);

        trie.put(&mut txn, &key_1, 9);
        trie.put(&mut txn, &key_2, 8);
        trie.put(&mut txn, &key_3, 7);
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

        trie.put(&mut txn, &key_1, 9);
        trie.put(&mut txn, &key_2, 8);
        trie.put(&mut txn, &key_3, 7);
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
}
