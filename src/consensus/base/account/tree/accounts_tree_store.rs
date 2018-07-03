use std::collections::HashMap;
use super::{AddressNibbles, AccountsTreeNode};
use std::fmt::Debug;
use lmdb_zero;
use beserial::{Serialize, Deserialize};
use std::io;
use lmdb_zero::traits::LmdbResultExt;

pub (in super) trait AccountsTreeStore: Debug {
    fn get(&self, key: &AddressNibbles) -> Option<AccountsTreeNode>;
    fn put(&mut self, node: &AccountsTreeNode);
    fn remove(&mut self, key: &AddressNibbles);
    fn get_root(&self) -> Option<AccountsTreeNode> {
        return self.get(&AddressNibbles::empty());
    }
}

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
}

impl AccountsTreeStore for VolatileAccountsTreeStore {
    fn get(&self, key: &AddressNibbles) -> Option<AccountsTreeNode> {
        if let Some(ref node) = self.store.get(key) {
            return Some((*node).clone());
        }
        return None;
    }

    fn put(&mut self, node: &AccountsTreeNode) {
        self.store.insert(node.prefix().to_owned(), node.to_owned());
    }

    fn remove(&mut self, key: &AddressNibbles) {
        self.store.remove(key);
    }
}

#[derive(Debug)]
pub (in super) struct PersistentAccountsTreeStore<'a> {
    env: &'a lmdb_zero::Environment,
    db: lmdb_zero::Database<'a>,
}

impl<'a> AccountsTreeStore for PersistentAccountsTreeStore<'a> {
    fn get(&self, key: &AddressNibbles) -> Option<AccountsTreeNode> {
        // Read some data in a transaction
        let txn = lmdb_zero::ReadTransaction::new(self.env).unwrap();
        let mut access = txn.access();
        // TODO: Essentially needs two copy operations: 1) Serialization, 2) Copy to DB
        let key: Vec<u8> = Vec::from(key);
        let result: Option<&[u8]> = access.get(&self.db, key.as_slice()).to_opt().unwrap();
        let mut c = io::Cursor::new(result?);
        return Deserialize::deserialize(&mut c).unwrap(); // TODO Handle errors here
    }

    fn put(&mut self, node: &AccountsTreeNode) {
        // Write some data in a transaction
        let txn = lmdb_zero::WriteTransaction::new(self.env).unwrap();
        {
            let mut access = txn.access();
            // TODO: Essentially needs two copy operations: 1) Serialization, 2) Copy to DB
            let key: Vec<u8> = Vec::from(node.prefix());
            let value: Vec<u8> = Vec::from(node);
            access.put(&self.db, key.as_slice(), value.as_slice(), lmdb_zero::put::Flags::empty()).unwrap();
        }
        // Commit the changes so they are visible to later transactions
        txn.commit().unwrap();
    }

    fn remove(&mut self, key: &AddressNibbles) {
        // Write some data in a transaction
        let txn = lmdb_zero::WriteTransaction::new(self.env).unwrap();
        {
            let mut access = txn.access();
            // TODO: Essentially needs two copy operations: 1) Serialization, 2) Copy to DB
            let key: Vec<u8> = Vec::from(key);
            access.del_key(&self.db, key.as_slice()).unwrap();
        }
        // Commit the changes so they are visible to later transactions
        txn.commit().unwrap();
    }
}

impl<'a> From<&'a AccountsTreeNode> for Vec<u8> {
    fn from(node: &'a AccountsTreeNode) -> Self {
        let mut v = Vec::with_capacity(node.serialized_size());
        node.serialize(&mut v).unwrap();
        return v;
    }
}

impl<'a> From<&'a AddressNibbles> for Vec<u8> {
    fn from(node: &'a AddressNibbles) -> Self {
        let mut v = Vec::with_capacity(node.serialized_size());
        node.serialize(&mut v).unwrap();
        return v;
    }
}
