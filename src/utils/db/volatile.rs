use super::*;
use std::collections::{HashMap, HashSet};
use std::cell::RefCell;

#[derive(Debug)]
pub struct VolatileEnvironment {
    env: RefCell<HashMap<String, HashMap<Vec<u8>, Vec<u8>>>>,
}

impl VolatileEnvironment {
    pub fn new() -> Environment {
        return Environment::Volatile(VolatileEnvironment { env: RefCell::new(HashMap::new()) });
    }
}

#[derive(Debug)]
pub struct VolatileDatabase {
    db_name: String,
}

impl VolatileEnvironment {
    pub(in super) fn open_database(&self, name: String) -> VolatileDatabase {
        self.env.borrow_mut().insert(name.clone(), HashMap::new());
        return VolatileDatabase {
            db_name: name
        };
    }
}


#[derive(Debug)]
pub struct VolatileReadTransaction<'env> {
    env: &'env VolatileEnvironment,
}

impl<'env> VolatileReadTransaction<'env> {
    pub(in super) fn new(env: &'env VolatileEnvironment) -> Self {
        return VolatileReadTransaction { env };
    }

    pub(in super) fn get<K, V>(&self, db: &VolatileDatabase, key: &K) -> Option<V> where K: AsDatabaseKey + ?Sized, V: FromDatabaseValue {
        let env = self.env.env.borrow();
        let result = env.get(&db.db_name)?.get(AsDatabaseKey::as_database_bytes(key).as_ref());
        return Some(FromDatabaseValue::copy_from_database(result?).unwrap());
    }
}

#[derive(Debug)]
pub struct VolatileWriteTransaction<'env> {
    env: &'env VolatileEnvironment,
    changes: HashMap<String, HashMap<Vec<u8>, Vec<u8>>>,
    removals: HashMap<String, HashSet<Vec<u8>>>,
}

impl<'env> VolatileWriteTransaction<'env> {
    pub(in super) fn new(env: &'env VolatileEnvironment) -> Self {
        return VolatileWriteTransaction {
            env,
            changes: HashMap::new(),
            removals: HashMap::new(),
        };
    }

    pub(in super) fn get<K, V>(&self, db: &VolatileDatabase, key: &K) -> Option<V> where K: AsDatabaseKey + ?Sized, V: FromDatabaseValue {
        let db_key = AsDatabaseKey::as_database_bytes(key);
        // First, check local changes.
        if let Some(changes) = self.changes.get(&db.db_name) {
            if let Some(result) = changes.get(db_key.as_ref()) {
                return Some(FromDatabaseValue::copy_from_database(result).unwrap());
            }
        }

        // Second, check removals.
        if let Some(removals) = self.removals.get(&db.db_name) {
            if removals.contains(db_key.as_ref()) {
                return None;
            }
        }

        // Third, check parent if exists.
        let env = self.env.env.borrow();
        let result = env.get(&db.db_name)?.get(AsDatabaseKey::as_database_bytes(key).as_ref());
        return Some(FromDatabaseValue::copy_from_database(result?).unwrap());
    }

    pub(in super) fn put<K, V>(&mut self, db: &VolatileDatabase, key: &K, value: &V) where K: AsDatabaseKey + ?Sized, V: IntoDatabaseValue + ?Sized {
        let mut db_value: Vec<u8> = vec![0; IntoDatabaseValue::database_byte_size(value)];
        IntoDatabaseValue::copy_into_database(value, &mut db_value);
        let db_key = AsDatabaseKey::as_database_bytes(key);

        if !self.changes.contains_key(&db.db_name) {
            self.changes.insert(db.db_name.clone(), HashMap::new());
        }
        let changes = self.changes.get_mut(&db.db_name).unwrap();
        changes.insert(db_key.to_vec(), db_value);

        if let Some(ref mut removals) = self.removals.get_mut(&db.db_name) {
            removals.remove(db_key.as_ref());
        }
    }

    pub(in super) fn remove<K>(&mut self, db: &VolatileDatabase, key: &K) where K: AsDatabaseKey + ?Sized {
        let db_key = AsDatabaseKey::as_database_bytes(key);

        if !self.removals.contains_key(&db.db_name) {
            self.removals.insert(db.db_name.clone(), HashSet::new());
        }
        let removals = self.removals.get_mut(&db.db_name).unwrap();
        removals.insert(db_key.to_vec());

        if let Some(ref mut changes) = self.changes.get_mut(&db.db_name) {
            changes.remove(db_key.as_ref());
        }
    }

    pub(in super) fn commit(mut self) {
        let mut env = self.env.env.borrow_mut();
        for (db_name, removals) in self.removals.iter() {
            let mut parent_db = env.get_mut(db_name).unwrap();
            for key in removals.iter() {
                parent_db.remove(key);
            }
        }

        for (db_name, changes) in self.changes.iter_mut() {
            let mut parent_db = env.get_mut(db_name).unwrap();
            parent_db.extend(changes.drain());
        }
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
}
