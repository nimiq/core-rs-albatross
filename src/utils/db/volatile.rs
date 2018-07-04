use super::*;
use std::collections::{HashMap, HashSet};
use std::sync::{RwLock, Mutex, Arc, Weak};

#[derive(Debug)]
struct ChangeSet {
    changes: HashMap<String, HashMap<Vec<u8>, Vec<u8>>>,
    removals: HashMap<String, HashSet<Vec<u8>>>,
}

impl ChangeSet {
    fn new() -> Self {
        return ChangeSet {
            changes: HashMap::new(),
            removals: HashMap::new(),
        };
    }

    fn invert_remove(&mut self, db: &str, key: &Vec<u8>, old_value: &Vec<u8>) {
        // Keep track of changes. Only store old_value, if we don't have any information about the key yet.
        let changes = self.changes.entry(db.to_string()).or_insert_with(|| HashMap::new());
        let removals = self.removals.entry(db.to_string()).or_insert_with(|| HashSet::new());

        if !changes.contains_key(key) && !removals.contains(key) {
            changes.insert(key.to_owned(), old_value.to_owned());
        }
    }

    fn invert_put(&mut self, db: &str, key: &Vec<u8>, old_value: &Option<Vec<u8>>) {
        // Keep track of changes. Only store removal, if we don't have any information about the key yet.
        let changes = self.changes.entry(db.to_string()).or_insert_with(|| HashMap::new());
        let removals = self.removals.entry(db.to_string()).or_insert_with(|| HashSet::new());

        if !changes.contains_key(key) && !removals.contains(key) {
            if let Some(ref old_value ) = old_value {
                changes.insert(key.to_owned(), old_value.to_owned());
            } else {
                removals.insert(key.to_owned());
            }
        }
    }
}

#[derive(Debug)]
pub struct VolatileEnvironment {
    env: RwLock<HashMap<String, HashMap<Vec<u8>, Vec<u8>>>>,
    read_txns: Mutex<Vec<Weak<Mutex<ChangeSet>>>>,
}

impl VolatileEnvironment {
    pub fn new() -> Environment {
        return Environment::Volatile(VolatileEnvironment {
            env: RwLock::new(HashMap::new()), // This lock is taken before reading/writing.
            read_txns: Mutex::new(Vec::new()), // This lock is only taken to create new read-only transactions.
        });
    }

    fn register_read_transaction(&self, change_set: Weak<Mutex<ChangeSet>>) {
        // Cleanup and add current changeset.
        let mut read_txns = self.read_txns.lock().unwrap();
        read_txns.retain(|change_set| change_set.upgrade().is_some());
        read_txns.push(change_set);
    }

    pub(in super) fn open_database(&self, name: String) -> VolatileDatabase {
        self.env.write().unwrap().insert(name.clone(), HashMap::new());
        return VolatileDatabase {
            db_name: name
        };
    }

    fn commit_change_set(&self, mut change_set: ChangeSet) {
        // Take lock first, update all changesets and block reads in the meanwhile.
        let mut env = self.env.write().unwrap();
        let mut read_txns = self.read_txns.lock().unwrap();

        // Cleanup read_txns first.
        read_txns.retain(|change_set| change_set.upgrade().is_some());

        // Update main store.
        for (db_name, mut removals) in change_set.removals.drain() {
            let mut parent_db = env.get_mut(&db_name).unwrap();
            for key in removals.drain() {
                let result = parent_db.remove(&key);

                // Update read-only transactions.
                if let Some(old_value) = result {
                    // Now iterate over read_txns.
                    for txn in read_txns.iter() {
                        let change_set = txn.upgrade().unwrap();
                        let mut change_set = change_set.lock().unwrap();
                        change_set.invert_remove(&db_name, &key, &old_value);
                    }
                }
            }
        }

        for (db_name, mut changes) in change_set.changes.drain() {
            let mut parent_db = env.get_mut(&db_name).unwrap();
            for (key, value) in changes.drain() {
                let old_value = parent_db.insert(key.clone(), value);

                // Update read-only transactions.
                for txn in read_txns.iter() {
                    let change_set = txn.upgrade().unwrap();
                    let mut change_set = change_set.lock().unwrap();
                    change_set.invert_put(&db_name, &key, &old_value);
                }
            }
        }
    }
}

#[derive(Debug)]
pub struct VolatileDatabase {
    db_name: String,
}


#[derive(Debug)]
pub struct VolatileReadTransaction<'env> {
    env: &'env VolatileEnvironment,
    change_set: Arc<Mutex<ChangeSet>>,
}

impl<'env> VolatileReadTransaction<'env> {
    pub(in super) fn new(env: &'env VolatileEnvironment) -> Self {
        // Add own changeset to environment.
        let change_set = Arc::new(Mutex::new(ChangeSet::new()));
        env.register_read_transaction(Arc::downgrade(&change_set));
        return VolatileReadTransaction { env, change_set };
    }

    pub(in super) fn get<K, V>(&self, db: &VolatileDatabase, key: &K) -> Option<V> where K: AsDatabaseKey + ?Sized, V: FromDatabaseValue {
        // Take lock first, so that we are sure no commit is ongoing right now.
        let env = self.env.env.read().unwrap();
        // Also guarded by the previous lock:
        let change_set = self.change_set.lock().unwrap();
        let changes = &change_set.changes;
        let removals = &change_set.removals;

        let db_key = AsDatabaseKey::as_database_bytes(key);
        // First, check local changes.
        if let Some(changes) = changes.get(&db.db_name) {
            if let Some(result) = changes.get(db_key.as_ref()) {
                return Some(FromDatabaseValue::copy_from_database(result).unwrap());
            }
        }

        // Second, check removals.
        if let Some(removals) = removals.get(&db.db_name) {
            if removals.contains(db_key.as_ref()) {
                return None;
            }
        }

        // Third, check parent if exists.
        let result = env.get(&db.db_name)?.get(AsDatabaseKey::as_database_bytes(key).as_ref());
        return Some(FromDatabaseValue::copy_from_database(result?).unwrap());
    }
}

#[derive(Debug)]
pub struct VolatileWriteTransaction<'env> {
    env: &'env VolatileEnvironment,
    change_set: ChangeSet,
}

impl<'env> VolatileWriteTransaction<'env> {
    pub(in super) fn new(env: &'env VolatileEnvironment) -> Self {
        return VolatileWriteTransaction {
            env,
            change_set: ChangeSet::new()
        };
    }

    pub(in super) fn get<K, V>(&self, db: &VolatileDatabase, key: &K) -> Option<V> where K: AsDatabaseKey + ?Sized, V: FromDatabaseValue {
        // Take lock first, so that we are sure no commit is ongoing right now.
        let env = self.env.env.read().unwrap();
        let changes = &self.change_set.changes;
        let removals = &self.change_set.removals;

        let db_key = AsDatabaseKey::as_database_bytes(key);
        // First, check local changes.
        if let Some(changes) = changes.get(&db.db_name) {
            if let Some(result) = changes.get(db_key.as_ref()) {
                return Some(FromDatabaseValue::copy_from_database(result).unwrap());
            }
        }

        // Second, check removals.
        if let Some(removals) = removals.get(&db.db_name) {
            if removals.contains(db_key.as_ref()) {
                return None;
            }
        }

        // Third, check parent if exists.
        let result = env.get(&db.db_name)?.get(AsDatabaseKey::as_database_bytes(key).as_ref());
        return Some(FromDatabaseValue::copy_from_database(result?).unwrap());
    }

    pub(in super) fn put<K, V>(&mut self, db: &VolatileDatabase, key: &K, value: &V) where K: AsDatabaseKey + ?Sized, V: IntoDatabaseValue + ?Sized {
        // Isolated puts don't need any lock.
        let changes = &mut self.change_set.changes;
        let removals = &mut self.change_set.removals;

        let mut db_value: Vec<u8> = vec![0; IntoDatabaseValue::database_byte_size(value)];
        IntoDatabaseValue::copy_into_database(value, &mut db_value);
        let db_key = AsDatabaseKey::as_database_bytes(key);

        if !changes.contains_key(&db.db_name) {
            changes.insert(db.db_name.clone(), HashMap::new());
        }
        let changes = changes.get_mut(&db.db_name).unwrap();
        changes.insert(db_key.to_vec(), db_value);

        if let Some(ref mut removals) = removals.get_mut(&db.db_name) {
            removals.remove(db_key.as_ref());
        }
    }

    pub(in super) fn remove<K>(&mut self, db: &VolatileDatabase, key: &K) where K: AsDatabaseKey + ?Sized {
        // Isolated removals don't need any lock.
        let changes = &mut self.change_set.changes;
        let removals = &mut self.change_set.removals;

        let db_key = AsDatabaseKey::as_database_bytes(key);

        if !removals.contains_key(&db.db_name) {
            removals.insert(db.db_name.clone(), HashSet::new());
        }
        let removals = removals.get_mut(&db.db_name).unwrap();
        removals.insert(db_key.to_vec());

        if let Some(ref mut changes) = changes.get_mut(&db.db_name) {
            changes.remove(db_key.as_ref());
        }
    }

    pub(in super) fn commit(self) {
        self.env.commit_change_set(self.change_set);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_can_save_basic_objects() {
        let env = VolatileEnvironment::new();
        {
            let db = env.open_database("test".to_string());

            // Read non-existent value.
            {
                let tx = ReadTransaction::new(&env);
                assert!(tx.get::<str, String>(&db, "test").is_none());
            }

            // Read non-existent value.
            let mut tx = WriteTransaction::new(&env);
            assert!(tx.get::<str, String>(&db, "test").is_none());

            // Write and read value.
            tx.put(&db, "test", "one");
            assert_eq!(tx.get::<str, String>(&db, "test"), Some("one".to_string()));
            // Overwrite and read value.
            tx.put(&db, "test", "two");
            assert_eq!(tx.get::<str, String>(&db, "test"), Some("two".to_string()));
            tx.commit();

            // Read value.
            let tx = ReadTransaction::new(&env);
            assert_eq!(tx.get::<str, String>(&db, "test"), Some("two".to_string()));
            tx.close();

            // Remove value.
            let mut tx = WriteTransaction::new(&env);
            tx.remove(&db, "test");
            assert!(tx.get::<str, String>(&db, "test").is_none());
            tx.commit();

            // Check removal.
            {
                let tx = ReadTransaction::new(&env);
                assert!(tx.get::<str, String>(&db, "test").is_none());
            }

            // Write and abort.
            let mut tx = WriteTransaction::new(&env);
            tx.put(&db, "test", "one");
            tx.abort();

            // Check aborted transaction.
            let tx = ReadTransaction::new(&env);
            assert!(tx.get::<str, String>(&db, "test").is_none());
        }

        env.drop_database().unwrap();
    }

    #[test]
    fn isolation_test() {
        let env = VolatileEnvironment::new();
        {
            let db = env.open_database("test".to_string());

            // Read non-existent value.
            let tx = ReadTransaction::new(&env);
            assert!(tx.get::<str, String>(&db, "test").is_none());

            // WriteTransaction.
            let mut txw = WriteTransaction::new(&env);
            assert!(txw.get::<str, String>(&db, "test").is_none());
            txw.put(&db, "test", "one");
            assert_eq!(txw.get::<str, String>(&db, "test"), Some("one".to_string()));

            // ReadTransaction should still have the old state.
            assert!(tx.get::<str, String>(&db, "test").is_none());

            // Commit WriteTransaction.
            txw.commit();

            // ReadTransaction should still have the old state.
            assert!(tx.get::<str, String>(&db, "test").is_none());

            // Have a new ReadTransaction read the new state.
            let tx2 = ReadTransaction::new(&env);
            assert_eq!(tx2.get::<str, String>(&db, "test"), Some("one".to_string()));
        }

        env.drop_database().unwrap();
    }
}
