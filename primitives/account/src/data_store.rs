use beserial::{Deserialize, Serialize};
use nimiq_database::{Transaction, WriteTransaction};
use nimiq_keys::Address;
use nimiq_primitives::key_nibbles::KeyNibbles;
use nimiq_trie::trie::MerkleRadixTrie;

/// An alias for the accounts tree.
pub type AccountsTrie = MerkleRadixTrie;

pub struct DataStore<'a> {
    tree: &'a AccountsTrie,
    prefix: KeyNibbles,
}

impl<'tree> DataStore<'tree> {
    pub fn new(tree: &'tree AccountsTrie, prefix: &Address) -> Self {
        DataStore {
            tree,
            prefix: KeyNibbles::from(prefix),
        }
    }

    pub fn get<T: Deserialize>(&self, txn: &Transaction, key: &KeyNibbles) -> Option<T> {
        self.tree
            .get(txn, &(&self.prefix + key))
            .expect("Tree must be complete")
    }

    pub fn put<T: Serialize>(&self, txn: &mut WriteTransaction, key: &KeyNibbles, value: T) {
        self.tree
            .put(txn, &(&self.prefix + key), value)
            .expect("Tree must be complete")
    }

    pub fn remove(&self, txn: &mut WriteTransaction, key: &KeyNibbles) {
        self.tree.remove(txn, &(&self.prefix + key))
    }

    pub fn read<'store, 'txn, 'env>(
        &'store self,
        txn: &'txn Transaction<'env>,
    ) -> DataStoreRead<'store, 'tree, 'txn, 'env> {
        DataStoreRead { store: self, txn }
    }

    pub fn write<'store, 'txn, 'env>(
        &'store self,
        txn: &'txn mut WriteTransaction<'env>,
    ) -> DataStoreWrite<'store, 'tree, 'txn, 'env> {
        DataStoreWrite { store: self, txn }
    }
}

pub struct DataStoreRead<'store, 'tree, 'txn, 'env> {
    store: &'store DataStore<'tree>,
    txn: &'txn Transaction<'env>,
}

impl<'store, 'tree, 'txn, 'env> DataStoreRead<'store, 'tree, 'txn, 'env> {
    pub fn get<T: Deserialize>(&self, key: &KeyNibbles) -> Option<T> {
        self.store.get(self.txn, key)
    }
}

pub struct DataStoreWrite<'store, 'tree, 'txn, 'env> {
    store: &'store DataStore<'tree>,
    txn: &'txn mut WriteTransaction<'env>,
}

impl<'store, 'tree, 'txn, 'env> DataStoreWrite<'store, 'tree, 'txn, 'env> {
    pub fn get<T: Deserialize>(&self, key: &KeyNibbles) -> Option<T> {
        self.store.get(self.txn, key)
    }

    pub fn put<T: Serialize>(&mut self, key: &KeyNibbles, value: T) {
        self.store.put(self.txn, key, value)
    }

    pub fn remove(&mut self, key: &KeyNibbles) {
        self.store.remove(self.txn, key)
    }
}

#[cfg(test)]
mod tests {
    use crate::data_store::DataStore;
    use crate::AccountsTrie;
    use nimiq_database::volatile::VolatileEnvironment;
    use nimiq_database::{ReadTransaction, WriteTransaction};
    use nimiq_primitives::policy::Policy;

    #[test]
    fn data_store_works() {
        let env = VolatileEnvironment::new(10).unwrap();
        let tree = AccountsTrie::new(env.clone(), "accounts_trie");
        let store = DataStore::new(&tree, &Policy::STAKING_CONTRACT_ADDRESS);

        let mut txn = WriteTransaction::new(&env);
        let mut write = store.write(&mut txn);

        let key_1 = "290d7f3".parse().unwrap();
        let key_2 = "290d252".parse().unwrap();

        assert_eq!(write.get::<i32>(&key_1), None);
        assert_eq!(write.get::<i32>(&key_2), None);

        write.put(&key_1, 1337);
        write.put(&key_2, 6969);

        assert_eq!(write.get(&key_1), Some(1337));
        assert_eq!(write.get(&key_2), Some(6969));

        write.remove(&key_1);

        assert_eq!(write.get::<i32>(&key_1), None);
        assert_eq!(write.get(&key_2), Some(6969));

        drop(write);
        txn.commit();

        let txn = ReadTransaction::new(&env);
        let read = store.read(&txn);

        assert_eq!(read.get::<i32>(&key_1), None);
        assert_eq!(read.get(&key_2), Some(6969));
    }
}
