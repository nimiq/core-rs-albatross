use std::collections::HashMap;
use super::{AddressNibbles, AccountsTreeNode};

#[derive(Debug)]
pub (in super) struct VolatileAccountsTreeStore {
    store: HashMap<AddressNibbles, AccountsTreeNode>
}

impl VolatileAccountsTreeStore {
    pub (in super) fn new() -> Self {
        return VolatileAccountsTreeStore {
            store: HashMap::new()
        };
    }

    pub (in super) fn get(&self, key: &AddressNibbles) -> Option<AccountsTreeNode> {
        if let Some(ref node) = self.store.get(key) {
            return Some((*node).clone());
        }
        return None;
    }

    pub (in super) fn put(&mut self, node: &AccountsTreeNode) {
        self.store.insert(node.prefix().to_owned(), node.to_owned());
    }

    pub (in super) fn remove(&mut self, key: &AddressNibbles) -> Option<AccountsTreeNode> {
        return self.store.remove(key);
    }

    pub (in super) fn get_root(&self) -> Option<AccountsTreeNode> {
        return self.get(&AddressNibbles::empty());
    }
}
