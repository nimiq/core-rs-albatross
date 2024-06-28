use nimiq_database::mdbx::MdbxReadTransaction;
use nimiq_keys::Address;
use nimiq_primitives::key_nibbles::KeyNibbles;
use nimiq_serde::{Deserialize, Serialize};
use nimiq_trie::{trie::TrieNodeIter, WriteTransactionProxy};

use crate::{
    accounts::AccountsTrieTable,
    data_store_ops::{DataStoreIterOps, DataStoreReadOps},
    AccountsTrie,
};

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

    pub fn get<T: Deserialize>(&self, txn: &MdbxReadTransaction, key: &KeyNibbles) -> Option<T> {
        self.tree
            .get(txn, &(&self.prefix + key))
            .expect("Tree must be complete")
    }

    pub fn put<T: Serialize>(&self, txn: &mut WriteTransactionProxy, key: &KeyNibbles, value: T) {
        self.tree
            .put(txn, &(&self.prefix + key), value)
            .expect("Tree must be complete")
    }

    pub fn remove(&self, txn: &mut WriteTransactionProxy, key: &KeyNibbles) {
        self.tree.remove(txn, &(&self.prefix + key))
    }

    pub fn read<'store, 'txn, 'env>(
        &'store self,
        txn: &'txn MdbxReadTransaction<'env>,
    ) -> DataStoreRead<'store, 'tree, 'txn, 'env> {
        DataStoreRead { store: self, txn }
    }

    pub fn write<'store, 'txn, 'txni, 'env>(
        &'store self,
        txn: &'txn mut WriteTransactionProxy<'txni, 'env>,
    ) -> DataStoreWrite<'store, 'tree, 'txn, 'txni, 'env> {
        DataStoreWrite { store: self, txn }
    }
}

pub struct DataStoreRead<'store, 'tree, 'txn, 'env> {
    store: &'store DataStore<'tree>,
    txn: &'txn MdbxReadTransaction<'env>,
}

impl<'store, 'tree, 'txn, 'env> DataStoreReadOps for DataStoreRead<'store, 'tree, 'txn, 'env> {
    fn get<T: Deserialize>(&self, key: &KeyNibbles) -> Option<T> {
        self.store.get(self.txn, key)
    }
}

impl<'store, 'tree, 'txn, 'env> DataStoreIterOps for DataStoreRead<'store, 'tree, 'txn, 'env> {
    type Iter<T: Deserialize> = TrieNodeIter<'txn, AccountsTrieTable, T>;

    fn iter<T: Deserialize>(&self, start_key: &KeyNibbles, end_key: &KeyNibbles) -> Self::Iter<T> {
        self.store.tree.iter_nodes(
            self.txn,
            &(&self.store.prefix + start_key),
            &(&self.store.prefix + end_key),
        )
    }
}

pub struct DataStoreWrite<'store, 'tree, 'txn, 'txni, 'env> {
    store: &'store DataStore<'tree>,
    txn: &'txn mut WriteTransactionProxy<'txni, 'env>,
}

impl<'store, 'tree, 'txn, 'txni, 'env> DataStoreWrite<'store, 'tree, 'txn, 'txni, 'env> {
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
    use nimiq_database::{
        mdbx::MdbxDatabase,
        traits::{Database, WriteTransaction},
    };
    use nimiq_primitives::policy::Policy;

    use crate::{
        accounts::AccountsTrieTable, data_store::DataStore, data_store_ops::DataStoreReadOps,
        AccountsTrie,
    };

    #[test]
    fn data_store_works() {
        let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
        let tree = AccountsTrie::new(&env, AccountsTrieTable);
        let store = DataStore::new(&tree, &Policy::STAKING_CONTRACT_ADDRESS);

        let mut txn = env.write_transaction();
        let mut trie_txn = (&mut txn).into();
        let mut write = store.write(&mut trie_txn);

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

        let txn = env.read_transaction();
        let read = store.read(&txn);

        assert_eq!(read.get::<i32>(&key_1), None);
        assert_eq!(read.get(&key_2), Some(6969));
    }
}
