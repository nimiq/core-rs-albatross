use std::marker::PhantomData;

use beserial::{Deserialize, Serialize};
use nimiq_database::{Database, Environment, Transaction, WriteTransaction};
use nimiq_hash::{Blake2bHash, Hash};

use crate::prefix_nibbles::PrefixNibbles;
use crate::trie_node::{TrieNode, NO_CHILDREN};
use crate::trie_proof::TrieProof;

#[derive(Debug)]
pub struct PatriciaTrie<A: Serialize + Deserialize + Clone> {
    db: Database,
    _value: PhantomData<A>,
}

impl<A: Serialize + Deserialize + Clone> PatriciaTrie<A> {
    /// Start a new Patricia Trie with the given Environment and the given name.
    pub fn new(env: Environment, name: &str) -> Self {
        let db = env.open_database(name.to_string());

        let tree = PatriciaTrie {
            db,
            _value: PhantomData,
        };

        let mut txn = WriteTransaction::new(&env);

        if tree.get_root(&txn).is_none() {
            let root = PrefixNibbles::empty();

            txn.put_reserve(
                &tree.db,
                &root,
                &TrieNode::<A>::new_branch(root.clone(), NO_CHILDREN),
            );
        }

        txn.commit();

        tree
    }

    /// Returns the root hash of the Patricia Trie.
    pub fn root_hash(&self, txn: &Transaction) -> Blake2bHash {
        self.get_root(txn).unwrap().hash()
    }

    /// Get the value at the given prefix. If there's no terminal node at the given prefix then it
    /// returns None.
    pub fn get(&self, txn: &Transaction, prefix: &PrefixNibbles) -> Option<A> {
        let node = txn.get(&self.db, prefix)?;

        match node {
            TrieNode::BranchNode { .. } => None,
            TrieNode::TerminalNode { value, .. } => Some(value),
        }
    }

    /// Insert a value into the Patricia Trie at the given prefix. If the prefix already exists then
    /// it will overwrite it.  You can't use this to check the existence of a given prefix.
    pub fn put(&self, txn: &mut WriteTransaction, prefix: &PrefixNibbles, value: A) {
        // Start by getting the root node.
        let mut cur_node = self
            .get_root(txn)
            .expect("Patricia Trie must have a root node!");

        // And initialize the root path.
        let mut root_path: Vec<TrieNode<A>> = vec![];

        // Go down the trie until you find where to put the new value.
        loop {
            // If the current node prefix no longer matches our prefix then we need to split the node.
            if !cur_node.prefix().is_prefix_of(prefix) {
                // Create and store the new node.
                let new_node = TrieNode::new_terminal(prefix.clone(), value);
                txn.put_reserve(&self.db, prefix, &new_node);

                // Create and store the new parent node.
                let new_parent =
                    TrieNode::<A>::new_branch(cur_node.prefix().common_prefix(prefix), NO_CHILDREN)
                        .put_child(cur_node.prefix(), cur_node.hash())
                        .unwrap()
                        .put_child(new_node.prefix(), new_node.hash())
                        .unwrap();
                txn.put_reserve(&self.db, new_parent.prefix(), &new_parent);

                // Push the parent node into the root path.
                root_path.push(new_parent);

                break;
            }

            // If the current node prefix is equal to our prefix, we have found an existing terminal
            // node with the given prefix. Update the value.
            if cur_node.prefix() == prefix {
                assert!(
                    cur_node.is_terminal(),
                    "We can't put a terminal node in the middle of a branch!"
                );

                // Update the node and store it.
                cur_node = cur_node.put_value(value).unwrap();
                txn.put_reserve(&self.db, prefix, &cur_node);

                // Push the node into the root path.
                root_path.push(cur_node);

                break;
            }

            // Try to find a child of the current node that matches our prefix.
            match cur_node.get_child_prefix(prefix) {
                // If no matching child exists, add a new child account node to the current node.
                None => {
                    // Create and store the new node.
                    let new_node = TrieNode::<A>::new_terminal(prefix.clone(), value);
                    txn.put_reserve(&self.db, prefix, &new_node);

                    // Update the parent node and store it.
                    cur_node = cur_node
                        .put_child(new_node.prefix(), new_node.hash())
                        .unwrap();
                    txn.put_reserve(&self.db, cur_node.prefix(), &cur_node);

                    // Push the parent node into the root path.
                    root_path.push(cur_node);

                    break;
                }
                // If there's a child, then we update the current node and the root path, and
                // continue down the trie.
                Some(child_prefix) => {
                    root_path.push(cur_node);
                    cur_node = txn.get(&self.db, &child_prefix).unwrap();
                }
            }
        }

        // Update the keys and hashes of the nodes that we modified.
        self.update_nodes(txn, root_path);
    }

    /// Removes the value in the Patricia Trie at the given prefix. If the prefix doesn't exists
    /// then this function just returns silently. You can't use this to check the existence of a
    /// given prefix.
    pub fn remove(&self, txn: &mut WriteTransaction, prefix: &PrefixNibbles) {
        // Start by getting the root node.
        let mut cur_node = self
            .get_root(txn)
            .expect("Patricia Trie must have a root node!");

        // And initialize the root path.
        let mut root_path: Vec<TrieNode<A>> = vec![];

        // Go down the trie until you find the prefix.
        loop {
            // If the current node prefix no longer matches our prefix then the prefix doesn't
            // exist and we end here.
            if !cur_node.prefix().is_prefix_of(prefix) {
                return;
            }

            // If the current node prefix is equal to our given prefix, we have found our node.
            // Remove the node.
            if cur_node.prefix() == prefix {
                assert!(cur_node.is_terminal(), "We can't remove a branch node!");

                // Remove the node from the database.
                txn.remove(&self.db, prefix);

                break;
            }

            // Try to find a child of the current node that matches our prefix.
            match cur_node.get_child_prefix(prefix) {
                // If no matching child exists, then the prefix doesn't exist and we end here.
                None => {
                    return;
                }
                // If there's a child, then we update the current node and the root path, and
                // continue down the trie.
                Some(child_prefix) => {
                    root_path.push(cur_node);
                    cur_node = txn.get(&self.db, &child_prefix).unwrap();
                }
            }
        }

        // Walk along the root path towards the root node, starting with the immediate predecessor
        // of the node specified by 'prefix', and update the nodes along the way.
        let mut child_prefix = prefix.clone();

        while let Some(mut parent_node) = root_path.pop() {
            // Remove the child from the parent node.
            parent_node = parent_node.remove_child(&child_prefix).unwrap();

            // Get the root address and the number of children of the node.
            let root_address = PrefixNibbles::empty();
            let num_children = parent_node.iter_children().count();

            // If the node has only a single child (and it isn't the root node), merge it with the
            // next node.
            if num_children == 1 && parent_node.prefix() != &root_address {
                // Remove the node from the database.
                txn.remove(&self.db, parent_node.prefix());

                // Get the node's only child and add it to the root path.
                let only_child_prefix = parent_node.prefix()
                    + &parent_node.iter_children().next().unwrap().suffix.clone();

                let only_child = txn.get(&self.db, &only_child_prefix).unwrap();

                root_path.push(only_child);

                // Update the keys and hashes of the rest of the root path.
                self.update_nodes(txn, root_path);

                return;
            }
            // If the node has any children, or it is the root node, we just store the
            // parent node in the database and update the keys and hashes of the rest of the root path.
            else if num_children > 0 || parent_node.prefix() == &root_address {
                txn.put_reserve(&self.db, parent_node.prefix(), &parent_node);

                self.update_nodes(txn, root_path);

                return;
            }
            // Otherwise, our node must have no children and not be the root node. In this case we
            // need to remove too, so we loop again.
            else {
                child_prefix = parent_node.prefix().clone();
            }
        }
    }

    // Starting prefix might or not exist, it still produces a proof.
    pub fn get_chunk(
        &self,
        txn: &Transaction,
        start: &PrefixNibbles,
        size: usize,
    ) -> Option<TrieProof<A>> {
        let mut chunk = Vec::new();

        let mut stack = vec![self.get_root(txn)?];

        while let Some(item) = stack.pop() {
            match item {
                TrieNode::BranchNode { children, prefix } => {
                    for child in children.iter().flatten().rev() {
                        let combined = &prefix + &child.suffix;
                        if combined.is_prefix_of(start) || *start <= combined {
                            stack.push(txn.get(&self.db, &combined)?);
                        }
                    }
                }
                TrieNode::TerminalNode { ref prefix, .. } => {
                    if start.len() < prefix.len() || start < prefix {
                        chunk.push(item);
                    }
                    if chunk.len() >= size {
                        break;
                    }
                }
            }
        }

        let chunk_prefixes = chunk.iter().map(|node| node.prefix().clone()).collect();

        self.get_accounts_proof(txn, chunk_prefixes)
    }

    // Does return leaves. Nodes in post order. If prefix doesn't exist it returns None. Non-existence
    // can be proven but requires us to state if the node is part of the trie or not.
    pub fn get_accounts_proof(
        &self,
        txn: &Transaction,
        mut prefixes: Vec<PrefixNibbles>,
    ) -> Option<TrieProof<A>> {
        // We sort the prefixes to simplify traversal in post order (leftmost prefixes first).
        prefixes.sort();

        // Initialize the vector that will contain the proof.
        let mut proof_nodes = Vec::new();

        // Initialize the pointer node, we will use it to go up and down the tree. We always start
        // at the root.
        let mut pointer_node = self
            .get_root(txn)
            .expect("Patricia Trie must have a root node!");

        // Initialize the root path with the root node.
        let mut root_path: Vec<TrieNode<A>> = vec![pointer_node.clone()];

        // Get the first prefix.
        let mut cur_prefix = prefixes
            .pop()
            .expect("There must be at least one prefix to prove!");

        // Iterate over all the prefixes that we wish to prove.
        loop {
            // Go down the trie until we find a node with our prefix or we can't go any further.
            loop {
                // If the prefix does not match, the requested prefix is not part of this trie. In
                // this case, we can't produce a proof so we terminate now.
                if !pointer_node.prefix().is_prefix_of(&cur_prefix) {
                    return None;
                }

                // If the prefix fully matches, we have found the requested node. We must check that
                // it is a terminal node, we don't want to prove branch nodes. If it is, we add it
                // to the root path.
                if pointer_node.prefix() == &cur_prefix {
                    if pointer_node.is_branch() {
                        return None;
                    }

                    root_path.push(pointer_node.clone());

                    break;
                }

                // Otherwise, try to find a child of the pointer node that matches our prefix.
                match pointer_node.get_child_prefix(&cur_prefix) {
                    // If no matching child exists, then the requested prefix is not part of this
                    // trie. Once again, we can't produce a proof so we terminate now.
                    None => {
                        return None;
                    }
                    // If there's a child, then we update the pointer node and the root path, and
                    // continue down the trie.
                    Some(child_prefix) => {
                        root_path.push(pointer_node.clone());
                        pointer_node = txn.get(&self.db, &child_prefix).unwrap();
                    }
                }
            }

            // Get the next prefix. If there's no next prefix then we get out of the loop.
            match prefixes.pop() {
                None => break,
                Some(prefix) => cur_prefix = prefix,
            }

            // Go up the root path until we get to a node that is a prefix to our current prefix.
            // Add the nodes you remove to the proof.
            while !pointer_node.prefix().is_prefix_of(&cur_prefix) {
                proof_nodes.push(pointer_node.clone());

                pointer_node = root_path
                    .pop()
                    .expect("Root path must contain at least the root node!");
            }
        }

        // Add the remaining nodes in the root path to the proof. Evidently they must be added in
        // the reverse order.
        root_path.reverse();
        proof_nodes.append(&mut root_path);

        // Return the proof.
        Some(TrieProof::new(proof_nodes))
    }

    /// Returns the root node, if there is one.
    fn get_root(&self, txn: &Transaction) -> Option<TrieNode<A>> {
        txn.get(&self.db, &PrefixNibbles::empty())
    }

    /// Updates the keys and the hashes for a chain of nodes. It assumes that the path starts at the
    /// root node and that each consecutive node is a child of the previous node.
    fn update_nodes(&self, txn: &mut WriteTransaction, mut root_path: Vec<TrieNode<A>>) {
        // Get the first node in the path.
        let mut child_node = root_path.pop().expect("Root path must not be empty!");

        // Go up the root path until you get to the root.
        while let Some(mut parent_node) = root_path.pop() {
            // Update and store the parent node.
            parent_node = parent_node
                .put_child(child_node.prefix(), child_node.hash())
                .unwrap();
            txn.put_reserve(&self.db, parent_node.prefix(), &parent_node);

            child_node = parent_node;
        }
    }
}

// #[cfg(test)]
// mod tests {
//     use std::convert::TryFrom;
//
//     use nimiq_account::Account;
//     use nimiq_primitives::coin::Coin;
//
//     use super::*;
//
//     #[test]
//     fn it_can_create_valid_chunk() {
//         let address1 =
//             Address::from(&hex::decode("0000000000000000000000000000000000000000").unwrap()[..]);
//         let account1 = Account::Basic(nimiq_account::BasicAccount {
//             balance: Coin::try_from(5).unwrap(),
//         });
//         let address2 =
//             Address::from(&hex::decode("1000000000000000000000000000000000000000").unwrap()[..]);
//         let account2 = Account::Basic(nimiq_account::BasicAccount {
//             balance: Coin::try_from(55).unwrap(),
//         });
//         let address3 =
//             Address::from(&hex::decode("1200000000000000000000000000000000000000").unwrap()[..]);
//         let account3 = Account::Basic(nimiq_account::BasicAccount {
//             balance: Coin::try_from(55555555).unwrap(),
//         });
//
//         let env = nimiq_database::volatile::VolatileEnvironment::new(10).unwrap();
//         let tree = AccountsTree::new(env.clone());
//         let mut txn = WriteTransaction::new(&env);
//
//         // Put accounts and check.
//         tree.put(&mut txn, &address1, account1);
//         tree.put(&mut txn, &address2, account2);
//         tree.put(&mut txn, &address3, account3);
//
//         let mut chunk = tree.get_chunk(&txn, "", 100).unwrap();
//         assert_eq!(chunk.len(), 3);
//         assert_eq!(chunk.verify(), true);
//     }
// }
