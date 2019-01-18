use super::{AccountsTreeNode, AddressNibbles, NO_CHILDREN};
use super::super::{Address, Account};
use crate::consensus::base::primitive::hash::{Hash, Blake2bHash};
use database::{Database, Transaction, WriteTransaction, Environment};

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
        // Find common prefix between node and new address.
        let common_prefix = node_prefix.common_prefix(&prefix);
        //println!("Common Prefix between {} and {}: {}", node_prefix.to_string(), prefix.to_string(), common_prefix.to_string());

        // If the node prefix does not fully match the new address, split the node.
        if common_prefix.len() != node_prefix.len() {
            // Insert the new account node.
            let new_child = AccountsTreeNode::new_terminal(prefix, account);
            txn.put_reserve(&self.db, new_child.prefix(), &new_child);

            // Insert the new parent node.
            let new_parent = AccountsTreeNode::new_branch(common_prefix, NO_CHILDREN)
                .with_child(&node_prefix, Blake2bHash::default()).unwrap()
                .with_child(new_child.prefix(), Blake2bHash::default()).unwrap();
            txn.put_reserve(&self.db, new_parent.prefix(), &new_parent);

            return self.update_keys_batch(txn, new_parent.prefix().clone(), root_path);
        }

        // If the commonPrefix is the specified address, we have found an (existing) node
        // with the given address. Update the account.
        if common_prefix == prefix {
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
