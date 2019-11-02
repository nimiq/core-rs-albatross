use std::marker::PhantomData;
use std::str::FromStr;

use account::AccountsTreeLeave;
use database::{Database, Environment, Transaction, WriteTransaction};
use hash::{Blake2bHash, Hash};
use keys::Address;
use tree_primitives::accounts_proof::AccountsProof;
use tree_primitives::accounts_tree_chunk::AccountsTreeChunk;
use tree_primitives::accounts_tree_node::{AccountsTreeNode, NO_CHILDREN};
use tree_primitives::address_nibbles::AddressNibbles;

#[derive(Debug)]
pub struct AccountsTree<A: AccountsTreeLeave> {
    db: Database,
    _account: PhantomData<A>,
}

impl<A: AccountsTreeLeave> AccountsTree<A> {
    const DB_NAME: &'static str = "accounts";

    pub fn new(env: Environment) -> Self {
        let db = env.open_database(Self::DB_NAME.to_string());
        let tree = AccountsTree { db, _account: PhantomData };

        let mut txn = WriteTransaction::new(&env);
        if tree.get_root(&txn).is_none() {
            let root = AddressNibbles::empty();
            txn.put_reserve(&tree.db, &root, &AccountsTreeNode::<A>::new_branch(root.clone(), NO_CHILDREN));
        }
        txn.commit();
        tree
    }

    pub fn put(&self, txn: &mut WriteTransaction, address: &Address, account: A) {
        self.put_batch(txn, address, account);
        self.finalize_batch(txn);
    }

    pub fn put_batch(&self, txn: &mut WriteTransaction, address: &Address, account: A) {
        if account.is_initial() && self.get(txn, address).is_none() {
            return;
        }

        // Insert account into the tree at address.
        let prefix = AddressNibbles::from(address);
        self.insert_batch(txn,AddressNibbles::empty(), prefix, account, Vec::new());
    }

    fn insert_batch(&self, txn: &mut WriteTransaction, node_prefix: AddressNibbles, prefix: AddressNibbles, account: A, mut root_path: Vec<AccountsTreeNode<A>>) {
        // If the node prefix does not fully match the new address, split the node.
        if !node_prefix.is_prefix_of(&prefix) {
            // Insert the new account node.
            let new_child = AccountsTreeNode::new_terminal(prefix.clone(), account);
            txn.put_reserve(&self.db, new_child.prefix(), &new_child);

            // Insert the new parent node.
            let new_parent = AccountsTreeNode::<A>::new_branch(node_prefix.common_prefix(&prefix), NO_CHILDREN)
                .with_child(&node_prefix, Blake2bHash::default()).unwrap()
                .with_child(new_child.prefix(), Blake2bHash::default()).unwrap();
            txn.put_reserve(&self.db, new_parent.prefix(), &new_parent);

            return self.update_keys_batch(txn, new_parent.prefix().clone(), root_path);
        }

        // If the commonPrefix is the specified address, we have found an (existing) node
        // with the given address. Update the account.
        if node_prefix == prefix {
            // XXX How does this generalize to more than one account type?
            // Special case: If the new balance is the initial balance
            // (i.e. balance=0, nonce=0), it is like the account never existed
            // in the first place. Delete the node in this case.
            if account.is_initial() {
                txn.remove(&self.db, &node_prefix);
                // We have already deleted the node, remove the subtree it was on.
                return self.prune_batch(txn, node_prefix, root_path);
            }

            // Update the account.
            let node: AccountsTreeNode<A> = txn.get(&self.db, &node_prefix).unwrap();
            let node = node.with_account(account).unwrap();
            txn.put_reserve(&self.db, node.prefix(), &node);

            return self.update_keys_batch(txn, node_prefix, root_path);
        }

        // If the node prefix matches and there are address bytes left, descend into
        // the matching child node if one exists.
        let node: AccountsTreeNode<A> = txn.get(&self.db, &node_prefix).unwrap();
        if let Some(child_prefix) = node.get_child_prefix(&prefix) {
            root_path.push(node);
            return self.insert_batch(txn, child_prefix, prefix, account, root_path);
        }

        // If no matching child exists, add a new child account node to the current node.
        let child = AccountsTreeNode::<A>::new_terminal(prefix, account);
        txn.put_reserve(&self.db, child.prefix(), &child);
        let node = node.with_child(child.prefix(), Blake2bHash::default()).unwrap();
        txn.put_reserve(&self.db, node.prefix(), &node);

        self.update_keys_batch(txn, node_prefix, root_path)
    }

    fn prune_batch(&self, txn: &mut WriteTransaction, prefix: AddressNibbles, mut root_path: Vec<AccountsTreeNode<A>>) {
        // Walk along the rootPath towards the root node starting with the
        // immediate predecessor of the node specified by 'prefix'.
        let mut tmp_prefix = prefix;
        while let Some(node) = root_path.pop() {
            let node = node.without_child(tmp_prefix).unwrap();
            let node_prefix = node.prefix();

            // If the node has only a single child, merge it with the next node.
            let root_address = AddressNibbles::empty();
            let num_children = node.iter_children().count();
            if num_children == 1 && node_prefix != &root_address {
                txn.remove(&self.db, node_prefix);

                let first_child = node.iter_children().nth(0).unwrap();
                return self.update_keys_batch(txn, node_prefix + &first_child.suffix, root_path);
            } else if num_children > 0 || node_prefix == &root_address {
                // Otherwise, if the node has children left, update it and all keys on the
                // remaining root path. Pruning finished.
                // XXX Special case: We start with an empty root node. Don't delete it.
                txn.put_reserve(&self.db, node.prefix(), &node);
                return self.update_keys_batch(txn, node_prefix.clone(), root_path);
            }

            tmp_prefix = node_prefix.clone();
        }
    }

    fn update_keys_batch(&self, txn: &mut WriteTransaction, prefix: AddressNibbles, mut root_path: Vec<AccountsTreeNode<A>>) {
        // Walk along the rootPath towards the root node starting with the
        // immediate predecessor of the node specified by 'prefix'.
        let mut tmp_prefix = &prefix;
        let mut node;
        while let Some(path_node) = root_path.pop() {
            node = path_node.with_child(tmp_prefix, Blake2bHash::default()).unwrap();
            txn.put_reserve(&self.db, node.prefix(), &node);
            tmp_prefix = node.prefix();
        }
    }

    pub fn finalize_batch(&self, txn: &mut WriteTransaction) {
        self.update_hashes(txn, &AddressNibbles::empty());
    }

    fn update_hashes(&self, txn: &mut WriteTransaction, node_key: &AddressNibbles) -> Blake2bHash {
        let mut node: AccountsTreeNode<A> = txn.get(&self.db, node_key).unwrap();
        if node.is_terminal() {
            return node.hash();
        }

        let zero_hash = Blake2bHash::default();
        // Compute sub hashes if necessary.
        for mut child in node.iter_children_mut() {
            if child.hash  == zero_hash {
                child.hash = self.update_hashes(txn, &(node_key + &child.suffix));
            }
        }
        txn.put_reserve(&self.db, node.prefix(), &node);
        node.hash()
    }

    pub fn get_accounts_proof(&self, txn: &Transaction, addresses: &[Address]) -> AccountsProof<A> {
        let mut prefixes = Vec::new();
        for address in addresses {
            prefixes.push(AddressNibbles::from(address));
        }
        // We sort the addresses to simplify traversal in post order (leftmost addresses first).
        prefixes.sort();

        let mut nodes = Vec::new();
        self.get_accounts_proof_rec(&txn, &self.get_root(txn).unwrap(), &prefixes, &mut nodes);
        AccountsProof::new(nodes)
    }

    fn get_accounts_proof_rec(&self, txn: &Transaction, node: &AccountsTreeNode<A>, prefixes: &[AddressNibbles], nodes: &mut Vec<AccountsTreeNode<A>>) -> bool {
        // For each prefix, descend the tree individually.
        let mut include_node = false;
        let mut i = 0;
        while i < prefixes.len() {
            let prefix = &prefixes[i];

            // If the prefix fully matches, we have found the requested node.
            // If the prefix does not fully match, the requested address is not part of this node.
            // Include the node in the proof nevertheless to prove that the account doesn't exist.
            if !node.prefix().is_prefix_of(prefix) || node.prefix() == prefix {
                include_node = true;
                i += 1;
                continue;
            }

            if let Some(child_prefix) = node.get_child_prefix(prefix) {
                let child_node: AccountsTreeNode<A> = txn.get(&self.db, &child_prefix).unwrap();

                // Group addresses with same prefix:
                // Because of our ordering, they have to be located next to the current prefix.
                // Hence, we iterate over the next prefixes, until we don't find commonalities anymore.
                // In the next main iteration we can skip those we already requested here.
                let mut sub_prefixes = vec![ prefixes[i].clone() ];
                // Find other prefixes to descend into this tree as well.
                for (j, prefix) in prefixes.iter().enumerate().skip(i+1) {
                    // Since we ordered prefixes, there can't be any other prefixes with commonalities.
                    if !child_prefix.is_prefix_of(prefix) {
                        break;
                    }
                    // But if there is a commonality, add it to the list.
                    sub_prefixes.push(prefix.clone());
                    // Move j forward. As soon as j is the last index which doesn't have commonalities,
                    // we continue from there in the next iteration.
                    i = j;
                }
                include_node = self.get_accounts_proof_rec(txn, &child_node, &sub_prefixes, nodes) || include_node;
            } else {
                // No child node exists with the requested prefix. Include the current node to prove the absence of the requested account.
                include_node = true;
                i += 1;
            }
            i += 1;
        }

        // If this branch contained at least one account, we add this node.
        if include_node {
            nodes.push(node.clone());
        }

        include_node
    }

    pub fn get(&self, txn: &Transaction, address: &Address) -> Option<A> {
        if let AccountsTreeNode::TerminalNode { account, .. } = txn.get(&self.db, &AddressNibbles::from(address))? {
            return Some(account);
        }
        None
    }

    pub(crate) fn get_chunk(&self, txn: &Transaction, start: &str, size: usize) -> Option<AccountsTreeChunk<A>> {
        let mut chunk = self.get_terminal_nodes(txn, &AddressNibbles::from_str(start).ok()?, size)?;
        let last_node = chunk.pop();
        let proof = if let Some(node) = last_node {
            self.get_accounts_proof(txn, &[node.prefix().to_address()?])
        } else {
            self.get_accounts_proof(txn, &[Address::from_str("ffffffffffffffffffffffffffffffffffffffff").ok()?])
        };
        Some(AccountsTreeChunk::new(chunk, proof))
    }

    pub(crate) fn get_terminal_nodes(&self, txn: &Transaction, start: &AddressNibbles, size: usize) -> Option<Vec<AccountsTreeNode<A>>> {
        let mut vec = Vec::new();
        let mut stack = Vec::new();
        stack.push(self.get_root(txn)?);
        while let Some(item) = stack.pop() {
            match item {
                AccountsTreeNode::BranchNode { children, prefix } => {
                    for child in children.iter().flatten().rev() {
                        let combined = &prefix + &child.suffix;
                        if combined.is_prefix_of(start) || *start <= combined {
                            stack.push(txn.get(&self.db, &combined)?);
                        }
                    }
                }
                AccountsTreeNode::TerminalNode { ref prefix, .. } => {
                    if start.len() < prefix.len() || start < prefix {
                        vec.push(item);
                    }
                    if vec.len() >= size {
                        return Some(vec);
                    }
                }
            }
        }
        Some(vec)
    }

    fn get_root(&self, txn: &Transaction) -> Option<AccountsTreeNode<A>> {
        txn.get(&self.db, &AddressNibbles::empty())
    }

    pub fn root_hash(&self, txn: &Transaction) -> Blake2bHash {
        let node = self.get_root(txn).unwrap();
        node.hash()
    }
}

#[cfg(test)]
mod tests {
    use std::convert::TryFrom;

    use account::Account;
    use nimiq_primitives::coin::Coin;

    use super::*;

    #[test]
    fn it_can_create_valid_chunk() {
        let address1 = Address::from(&hex::decode("0000000000000000000000000000000000000000").unwrap()[..]);
        let account1 = Account::Basic(account::BasicAccount { balance: Coin::try_from(5).unwrap() });
        let address2 = Address::from(&hex::decode("1000000000000000000000000000000000000000").unwrap()[..]);
        let account2 = Account::Basic(account::BasicAccount { balance: Coin::try_from(55).unwrap() });
        let address3 = Address::from(&hex::decode("1200000000000000000000000000000000000000").unwrap()[..]);
        let account3 = Account::Basic(account::BasicAccount { balance: Coin::try_from(55555555).unwrap() });

        let env = database::volatile::VolatileEnvironment::new(10).unwrap();
        let tree = AccountsTree::new(env.clone());
        let mut txn = WriteTransaction::new(&env);

        // Put accounts and check.
        tree.put(&mut txn, &address1, account1.clone());
        tree.put(&mut txn, &address2, account2.clone());
        tree.put(&mut txn, &address3, account3.clone());

        let mut chunk = tree.get_chunk(&txn, "", 100).unwrap();
        assert_eq!(chunk.len(), 3);
        assert_eq!(chunk.verify(), true);
    }
}
