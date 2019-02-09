use crate::accounts_proof::AccountsProof;
use database::{Database, Transaction, WriteTransaction, Environment};
use hash::{Hash, Blake2bHash};
use keys::Address;
use primitives::account::Account;
use std::cmp::min;
use std::sync::Arc;
use super::{AccountsTreeNode, AddressNibbles, NO_CHILDREN};

#[derive(Debug)]
pub struct AccountsTree<'env> {
    db: Database<'env>
}

impl<'env> AccountsTree<'env> {
    const DB_NAME: &'static str = "accounts";

    pub fn new(env: &'env Environment) -> Self {
        let db = env.open_database(Self::DB_NAME.to_string());
        let tree = AccountsTree { db };

        let mut txn = WriteTransaction::new(env);
        if tree.get_root(&txn).is_none() {
            let root = AddressNibbles::empty();
            txn.put_reserve(&tree.db, &root, &AccountsTreeNode::new_branch(root.clone(), NO_CHILDREN));
        }
        txn.commit();
        return tree;
    }

    pub fn put(&self, txn: &mut WriteTransaction, address: &Address, account: Account) {
        self.put_batch(txn, address, account);
        self.finalize_batch(txn);
    }

    pub fn put_batch(&self, txn: &mut WriteTransaction, address: &Address, account: Account) {
        if account.is_initial() && self.get(txn, address).is_none() {
            return;
        }

        // Insert account into the tree at address.
        let prefix = AddressNibbles::from(address);
        self.insert_batch(txn,AddressNibbles::empty(), prefix, account, Vec::new());
    }

    fn insert_batch(&self, txn: &mut WriteTransaction, node_prefix: AddressNibbles, prefix: AddressNibbles, account: Account, mut root_path: Vec<AccountsTreeNode>) {
        // If the node prefix does not fully match the new address, split the node.
        if !node_prefix.is_prefix_of(&prefix) {
            // Insert the new account node.
            let new_child = AccountsTreeNode::new_terminal(prefix.clone(), account);
            txn.put_reserve(&self.db, new_child.prefix(), &new_child);

            // Insert the new parent node.
            let new_parent = AccountsTreeNode::new_branch(node_prefix.common_prefix(&prefix), NO_CHILDREN)
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
            let node: AccountsTreeNode = txn.get(&self.db, &node_prefix).unwrap();
            let node = node.with_account(account).unwrap();
            txn.put_reserve(&self.db, node.prefix(), &node);

            return self.update_keys_batch(txn, node_prefix, root_path);
        }

        // If the node prefix matches and there are address bytes left, descend into
        // the matching child node if one exists.
        let node: AccountsTreeNode = txn.get(&self.db, &node_prefix).unwrap();
        if let Some(child_prefix) = node.get_child_prefix(&prefix) {
            root_path.push(node);
            return self.insert_batch(txn, child_prefix, prefix, account, root_path);
        }

        // If no matching child exists, add a new child account node to the current node.
        let child = AccountsTreeNode::new_terminal(prefix, account);
        txn.put_reserve(&self.db, child.prefix(), &child);
        let node = node.with_child(child.prefix(), Blake2bHash::default()).unwrap();
        txn.put_reserve(&self.db, node.prefix(), &node);

        return self.update_keys_batch(txn, node_prefix, root_path);
    }

    fn prune_batch(&self, txn: &mut WriteTransaction, prefix: AddressNibbles, mut root_path: Vec<AccountsTreeNode>) {
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

    fn update_keys_batch(&self, txn: &mut WriteTransaction, prefix: AddressNibbles, mut root_path: Vec<AccountsTreeNode>) {
        // Walk along the rootPath towards the root node starting with the
        // immediate predecessor of the node specified by 'prefix'.
        let mut tmp_prefix = prefix;
        while let Some(node) = root_path.pop() {
            let node = node.with_child(&tmp_prefix, Blake2bHash::default()).unwrap();
            txn.put_reserve(&self.db, node.prefix(), &node);
            tmp_prefix = node.prefix().clone(); // TODO: can we get rid of the clone here?
        }
    }

    pub fn finalize_batch(&self, txn: &mut WriteTransaction) {
        self.update_hashes(txn, &AddressNibbles::empty());
    }

    fn update_hashes(&self, txn: &mut WriteTransaction, node_key: &AddressNibbles) -> Blake2bHash {
        let mut node: AccountsTreeNode = txn.get(&self.db, node_key).unwrap();
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
        return node.hash();
    }

    pub fn get_accounts_proof(&self, txn: &Transaction, addresses: &Vec<Address>) -> AccountsProof {
        let mut prefixes = Vec::new();
        for address in addresses {
            prefixes.push(AddressNibbles::from(address));
        }
        // We sort the addresses to simplify traversal in post order (leftmost addresses first).
        prefixes.sort();

        let mut nodes = Vec::new();
        self.get_accounts_proof_rec(&txn, &self.get_root(txn).unwrap(), &prefixes, &mut nodes);
        return AccountsProof::new(nodes);
    }

    fn get_accounts_proof_rec(&self, txn: &Transaction, node: &AccountsTreeNode, prefixes: &Vec<AddressNibbles>, nodes: &mut Vec<AccountsTreeNode>) -> bool {
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
                let mut child_node: AccountsTreeNode = txn.get(&self.db, &child_prefix).unwrap();

                // Group addresses with same prefix:
                // Because of our ordering, they have to be located next to the current prefix.
                // Hence, we iterate over the next prefixes, until we don't find commonalities anymore.
                // In the next main iteration we can skip those we already requested here.
                let mut sub_prefixes = vec![ prefixes[0].clone() ];
                // Find other prefixes to descend into this tree as well.
                for j in i+1..prefixes.len() {
                    // Since we ordered prefixes, there can't be any other prefixes with commonalities.
                    if !child_prefix.is_prefix_of(&prefixes[j]) {
                        break;
                    }
                    // But if there is a commonality, add it to the list.
                    sub_prefixes.push(prefixes[j].clone());
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

        return include_node;
    }

    pub fn get(&self, txn: &Transaction, address: &Address) -> Option<Account> {
        if let AccountsTreeNode::TerminalNode { account, .. } = txn.get(&self.db, &AddressNibbles::from(address))? {
            return Some(account);
        }
        return None;
    }

    fn get_root(&self, txn: &Transaction) -> Option<AccountsTreeNode> {
        let node = txn.get(&self.db, &AddressNibbles::empty());
        return node;
    }

    pub fn root_hash(&self, txn: &Transaction) -> Blake2bHash {
        let node = self.get_root(txn).unwrap();
        return node.hash();
    }
}
